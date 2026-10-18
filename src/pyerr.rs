//! Python's error texts and file conventions: `str(OSError)`, codec errors,
//! `Path.read_text` newline handling, `pathlib` path strings and `resolve()`.

use std::io;
use std::path::{Component, Path, PathBuf};

use crate::pystr::repr;

/// Which Python call raised the error decides its text on Windows: `open()`
/// goes through the C runtime and reports errno values, while `os.stat`,
/// `os.replace`, `os.mkdir` and friends report Windows error codes.
#[derive(Clone, Copy, PartialEq)]
pub enum Call {
    Open,
    Os,
}

fn errno_text(errno: i32) -> &'static str {
    match errno {
        1 => "Operation not permitted",
        2 => "No such file or directory",
        5 => "Input/output error",
        9 => "Bad file descriptor",
        13 => "Permission denied",
        16 => "Device or resource busy",
        17 => "File exists",
        20 => "Not a directory",
        21 => "Is a directory",
        22 => "Invalid argument",
        24 => "Too many open files",
        27 => "File too large",
        28 => "No space left on device",
        30 => "Read-only file system",
        36 => "File name too long",
        39 => "Directory not empty",
        40 => "Too many levels of symbolic links",
        _ => "Unknown error",
    }
}

#[cfg(windows)]
fn windows_errno(code: i32) -> i32 {
    // The C runtime's _dosmaperr table, for the codes open() can meet.
    match code {
        2 | 3 | 15 | 18 | 53 | 161 => 2,
        5 | 19 | 32 | 33 | 65 | 82 | 108 | 132 | 267 => 13,
        4 => 24,
        6 => 9,
        80 | 183 => 17,
        112 => 28,
        145 => 41,
        _ => 22,
    }
}

#[cfg(windows)]
fn windows_message(code: i32) -> String {
    let text = io::Error::from_raw_os_error(code).to_string();
    let text = match text.rfind(" (os error ") {
        Some(index) => text[..index].to_string(),
        None => text,
    };
    text.trim_end_matches(|c: char| c == '.' || c.is_whitespace())
        .to_string()
}

fn prefix(err: &io::Error, call: Call) -> String {
    let Some(code) = err.raw_os_error() else {
        return err.to_string();
    };
    #[cfg(windows)]
    {
        if call == Call::Open {
            let errno = windows_errno(code);
            return format!("[Errno {errno}] {}", errno_text(errno));
        }
        format!("[WinError {code}] {}", windows_message(code))
    }
    #[cfg(not(windows))]
    {
        let _ = call;
        let text = errno_text(code);
        let text = if text == "Unknown error" {
            let full = err.to_string();
            match full.rfind(" (os error ") {
                Some(index) => full[..index].to_string(),
                None => full,
            }
        } else {
            text.to_string()
        };
        format!("[Errno {code}] {text}")
    }
}

/// `str(OSError)` with one filename.
pub fn os_error(err: &io::Error, call: Call, path: &Path) -> String {
    if err.raw_os_error().is_none() {
        return err.to_string();
    }
    format!("{}: {}", prefix(err, call), repr(&path_str(path)))
}

/// `str(OSError)` with two filenames, as `os.replace` raises.
pub fn os_error2(err: &io::Error, call: Call, src: &Path, dst: &Path) -> String {
    if err.raw_os_error().is_none() {
        return err.to_string();
    }
    format!(
        "{}: {} -> {}",
        prefix(err, call),
        repr(&path_str(src)),
        repr(&path_str(dst))
    )
}

/// The error Python's `open()` gives for a directory.
pub fn directory_open_error(path: &Path) -> String {
    let errno = if cfg!(windows) { 13 } else { 21 };
    format!(
        "[Errno {errno}] {}: {}",
        errno_text(errno),
        repr(&path_str(path))
    )
}

/// Python's text for a failed decode, as `str(UnicodeDecodeError)`.
pub fn decode_error(codec: &str, data: &[u8], start: usize, end: usize, reason: &str) -> String {
    if end - start == 1 {
        format!(
            "'{codec}' codec can't decode byte 0x{:02x} in position {start}: {reason}",
            data[start]
        )
    } else {
        format!(
            "'{codec}' codec can't decode bytes in position {start}-{}: {reason}",
            end - 1
        )
    }
}

/// A decode failure with the fields `_cmd_import` reports.
pub struct DecodeFailure {
    pub message: String,
    pub reason: String,
    pub start: usize,
}

/// Strict UTF-8 decoding with Python's error text.
pub fn decode_utf8(data: &[u8]) -> Result<String, DecodeFailure> {
    match std::str::from_utf8(data) {
        Ok(text) => Ok(text.to_string()),
        Err(err) => {
            let start = err.valid_up_to();
            let (end, reason) = match err.error_len() {
                None => (data.len(), "unexpected end of data"),
                Some(len) => {
                    let lead = data[start];
                    let reason = if (0xc2..=0xf4).contains(&lead) {
                        "invalid continuation byte"
                    } else {
                        "invalid start byte"
                    };
                    (start + len, reason)
                }
            };
            Err(DecodeFailure {
                message: decode_error("utf-8", data, start, end, reason),
                reason: reason.into(),
                start,
            })
        }
    }
}

/// Strict UTF-16 decoding with Python's error text.
pub fn decode_utf16(data: &[u8], big_endian: bool, codec: &str) -> Result<String, DecodeFailure> {
    let fail = |start: usize, end: usize, reason: &str| DecodeFailure {
        message: decode_error(codec, data, start, end, reason),
        reason: reason.into(),
        start,
    };
    let unit = |i: usize| {
        let pair = [data[i], data[i + 1]];
        if big_endian {
            u16::from_be_bytes(pair)
        } else {
            u16::from_le_bytes(pair)
        }
    };
    let mut out = String::with_capacity(data.len() / 2);
    let mut i = 0;
    while i + 1 < data.len() {
        let u = unit(i);
        if !(0xd800..=0xdfff).contains(&u) {
            out.push(char::from_u32(u32::from(u)).unwrap_or('\u{fffd}'));
            i += 2;
            continue;
        }
        if u >= 0xdc00 {
            return Err(fail(i, i + 2, "illegal encoding"));
        }
        if i + 3 >= data.len() {
            return Err(fail(i, data.len(), "unexpected end of data"));
        }
        let low = unit(i + 2);
        if !(0xdc00..=0xdfff).contains(&low) {
            return Err(fail(i, i + 2, "illegal UTF-16 surrogate"));
        }
        let cp = 0x10000 + ((u32::from(u) - 0xd800) << 10) + (u32::from(low) - 0xdc00);
        out.push(char::from_u32(cp).unwrap_or('\u{fffd}'));
        i += 4;
    }
    if i < data.len() {
        return Err(fail(i, data.len(), "truncated data"));
    }
    Ok(out)
}

fn decode_utf32(data: &[u8], big_endian: bool, codec: &str) -> Result<String, String> {
    let mut out = String::new();
    let mut i = 0;
    while i + 3 < data.len() {
        let bytes = [data[i], data[i + 1], data[i + 2], data[i + 3]];
        let cp = if big_endian {
            u32::from_be_bytes(bytes)
        } else {
            u32::from_le_bytes(bytes)
        };
        match char::from_u32(cp) {
            Some(c) => out.push(c),
            None => {
                return Err(decode_error(
                    codec,
                    data,
                    i,
                    i + 4,
                    "code point not in range(0x110000)",
                ));
            }
        }
        i += 4;
    }
    if i < data.len() {
        return Err(decode_error(codec, data, i, data.len(), "truncated data"));
    }
    Ok(out)
}

/// `json.loads(bytes)` decoding: `detect_encoding`, then decode.
pub fn decode_json_bytes(b: &[u8]) -> Result<String, String> {
    let encoding = if b.starts_with(&[0, 0, 0xfe, 0xff]) || b.starts_with(&[0xff, 0xfe, 0, 0]) {
        "utf-32"
    } else if b.starts_with(&[0xfe, 0xff]) || b.starts_with(&[0xff, 0xfe]) {
        "utf-16"
    } else if b.starts_with(&[0xef, 0xbb, 0xbf]) {
        "utf-8-sig"
    } else if b.len() >= 4 {
        if b[0] == 0 {
            if b[1] != 0 { "utf-16-be" } else { "utf-32-be" }
        } else if b[1] == 0 {
            if b[2] != 0 || b[3] != 0 {
                "utf-16-le"
            } else {
                "utf-32-le"
            }
        } else {
            "utf-8"
        }
    } else if b.len() == 2 {
        if b[0] == 0 {
            "utf-16-be"
        } else if b[1] == 0 {
            "utf-16-le"
        } else {
            "utf-8"
        }
    } else {
        "utf-8"
    };
    match encoding {
        "utf-8" => decode_utf8(b).map_err(|f| f.message),
        "utf-8-sig" => decode_utf8(&b[3..]).map_err(|f| f.message),
        "utf-16" => {
            let big = b.starts_with(&[0xfe, 0xff]);
            decode_utf16(&b[2..], big, "utf-16").map_err(|f| f.message)
        }
        "utf-16-be" => decode_utf16(b, true, "utf-16-be").map_err(|f| f.message),
        "utf-16-le" => decode_utf16(b, false, "utf-16-le").map_err(|f| f.message),
        "utf-32" => {
            let big = b.starts_with(&[0, 0, 0xfe, 0xff]);
            decode_utf32(&b[4..], big, "utf-32")
        }
        "utf-32-be" => decode_utf32(b, true, "utf-32-be"),
        _ => decode_utf32(b, false, "utf-32-le"),
    }
}

/// Universal-newline translation, as text-mode reads apply.
pub fn universal_newlines(text: &str) -> String {
    if !text.contains('\r') {
        return text.to_string();
    }
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// Text-mode write translation: `\n` becomes the platform line separator.
pub fn native_newlines(text: &str) -> String {
    if cfg!(windows) {
        text.replace('\n', "\r\n")
    } else {
        text.to_string()
    }
}

/// A failed read: Python's error text, and whether it was FileNotFoundError.
#[derive(Debug, Clone)]
pub struct ReadFail {
    pub not_found: bool,
    pub message: String,
}

impl From<ReadFail> for String {
    fn from(fail: ReadFail) -> String {
        fail.message
    }
}

/// `path.read_text(encoding="utf-8")`, with Python's error text.
pub fn read_text(path: &Path) -> Result<String, ReadFail> {
    let data = read_bytes(path)?;
    decode_utf8(&data)
        .map(|text| universal_newlines(&text))
        .map_err(|f| ReadFail {
            not_found: false,
            message: f.message,
        })
}

/// `path.read_bytes()`, with Python's error text.
pub fn read_bytes(path: &Path) -> Result<Vec<u8>, ReadFail> {
    if path.is_dir() {
        return Err(ReadFail {
            not_found: false,
            message: directory_open_error(path),
        });
    }
    std::fs::read(path).map_err(|e| ReadFail {
        not_found: e.kind() == io::ErrorKind::NotFound,
        message: os_error(&e, Call::Open, path),
    })
}

/// `path.mkdir(parents=True, exist_ok=True)`, with Python's error text.
pub fn mkdir_parents(path: &Path) -> Result<(), String> {
    match std::fs::create_dir(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            let parent = path
                .parent()
                .filter(|p| *p != path && !p.as_os_str().is_empty());
            match parent {
                Some(parent) => {
                    mkdir_parents(parent)?;
                    match std::fs::create_dir(path) {
                        Ok(()) => Ok(()),
                        Err(e) if path.is_dir() && e.kind() == io::ErrorKind::AlreadyExists => {
                            Ok(())
                        }
                        Err(e) => Err(os_error(&e, Call::Os, path)),
                    }
                }
                None => Err(os_error(&e, Call::Os, path)),
            }
        }
        Err(_) if path.is_dir() => Ok(()),
        Err(e) => Err(os_error(&e, Call::Os, path)),
    }
}

/// `str(path)`: Rust keeps the separators it was given, so a path that came
/// from the command line keeps them too; pathlib has normalised those already.
pub fn path_str(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// `str(Path(text))`: separators normalised, `.` parts and trailing
/// separators dropped, as pathlib does on this platform.
pub fn pathlib_normalise(text: &str) -> String {
    if text.is_empty() {
        return ".".into();
    }
    if cfg!(windows) {
        let text = text.replace('/', "\\");
        let (drive, rest) = split_windows_drive(&text);
        let rooted = rest.starts_with('\\');
        let parts: Vec<&str> = rest
            .split('\\')
            .filter(|part| !part.is_empty() && *part != ".")
            .collect();
        let mut out = drive.to_string();
        if rooted {
            out.push('\\');
        }
        out.push_str(&parts.join("\\"));
        if out.is_empty() { ".".into() } else { out }
    } else {
        let leading = if text.starts_with("//") && !text.starts_with("///") {
            "//"
        } else if text.starts_with('/') {
            "/"
        } else {
            ""
        };
        let parts: Vec<&str> = text
            .split('/')
            .filter(|part| !part.is_empty() && *part != ".")
            .collect();
        let out = format!("{leading}{}", parts.join("/"));
        if out.is_empty() { ".".into() } else { out }
    }
}

fn split_windows_drive(text: &str) -> (&str, &str) {
    let bytes = text.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        return text.split_at(2);
    }
    if let Some(rest) = text.strip_prefix("\\\\") {
        // UNC: \\server\share
        let mut parts = rest.splitn(3, '\\');
        let server = parts.next().unwrap_or("");
        let share = parts.next().unwrap_or("");
        if !server.is_empty() && !share.is_empty() {
            let len = 2 + server.len() + 1 + share.len();
            return text.split_at(len);
        }
    }
    ("", text)
}

/// `base / "a/b"`: each part joined with the platform separator.
pub fn join_rel(base: &Path, relative: &str) -> PathBuf {
    let mut path = base.to_path_buf();
    for part in relative.split('/') {
        if !part.is_empty() && part != "." {
            path.push(part);
        }
    }
    path
}

/// `os.path.normpath` on an absolute path.
pub fn normpath(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                let at_root = matches!(
                    out.components().next_back(),
                    None | Some(Component::RootDir | Component::Prefix(_))
                );
                if !at_root {
                    out.pop();
                }
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn absolute(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    }
}

#[cfg(windows)]
fn strip_verbatim(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    if let Some(rest) = text.strip_prefix(r"\\?\") {
        let bytes = rest.as_bytes();
        if bytes.len() >= 2 && bytes[1] == b':' {
            return PathBuf::from(rest);
        }
    }
    path
}

/// `Path.resolve()` (non-strict `os.path.realpath`).
pub fn resolve(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        let path = normpath(&absolute(path));
        if let Ok(found) = std::fs::canonicalize(&path) {
            return strip_verbatim(found);
        }
        // Resolve the deepest existing ancestor and keep the missing tail.
        let mut tail = Vec::new();
        let mut head = path.clone();
        loop {
            if let Ok(found) = std::fs::canonicalize(&head) {
                let mut out = strip_verbatim(found);
                for part in tail.iter().rev() {
                    out.push(part);
                }
                return out;
            }
            match (head.file_name().map(|n| n.to_os_string()), head.parent()) {
                (Some(name), Some(parent)) => {
                    tail.push(name);
                    head = parent.to_path_buf();
                }
                _ => return path,
            }
        }
    }
    #[cfg(not(windows))]
    {
        posix_realpath(&absolute(path))
    }
}

#[cfg(not(windows))]
fn posix_realpath(path: &Path) -> PathBuf {
    fn walk(path: &Path, resolved: &mut PathBuf, seen: &mut Vec<PathBuf>, depth: usize) {
        for component in path.components() {
            match component {
                Component::RootDir => *resolved = PathBuf::from("/"),
                Component::CurDir | Component::Prefix(_) => {}
                Component::ParentDir => {
                    resolved.pop();
                }
                Component::Normal(name) => {
                    let candidate = resolved.join(name);
                    match std::fs::symlink_metadata(&candidate) {
                        Ok(meta) if meta.file_type().is_symlink() && depth < 40 => {
                            if seen.contains(&candidate) {
                                *resolved = candidate;
                                continue;
                            }
                            let Ok(target) = std::fs::read_link(&candidate) else {
                                *resolved = candidate;
                                continue;
                            };
                            seen.push(candidate);
                            if target.is_absolute() {
                                *resolved = PathBuf::from("/");
                            }
                            walk(&target, resolved, seen, depth + 1);
                            seen.pop();
                        }
                        _ => *resolved = candidate,
                    }
                }
            }
        }
    }
    let mut resolved = PathBuf::from("/");
    walk(path, &mut resolved, &mut Vec::new(), 0);
    resolved
}

/// `path.is_relative_to(root)` for resolved paths.
pub fn is_relative_to(path: &Path, root: &Path) -> bool {
    path.starts_with(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf_errors_match_python() {
        assert_eq!(
            decode_utf8(b"ab\xff").err().unwrap().message,
            "'utf-8' codec can't decode byte 0xff in position 2: invalid start byte"
        );
        assert_eq!(
            decode_utf8(b"ab\xe2\x82").err().unwrap().message,
            "'utf-8' codec can't decode bytes in position 2-3: unexpected end of data"
        );
        assert_eq!(
            decode_utf8(b"ab\xe2\x28").err().unwrap().message,
            "'utf-8' codec can't decode byte 0xe2 in position 2: invalid continuation byte"
        );
        assert_eq!(
            decode_utf16(b"a\x00b", false, "utf-16-le")
                .err()
                .unwrap()
                .message,
            "'utf-16-le' codec can't decode byte 0x62 in position 2: truncated data"
        );
        assert_eq!(
            decode_utf16(b"\x00\xd8a\x00", false, "utf-16-le")
                .err()
                .unwrap()
                .message,
            "'utf-16-le' codec can't decode bytes in position 0-1: illegal UTF-16 surrogate"
        );
        assert_eq!(
            decode_utf16(b"\x00\xdca\x00", false, "utf-16-le")
                .err()
                .unwrap()
                .message,
            "'utf-16-le' codec can't decode bytes in position 0-1: illegal encoding"
        );
        assert_eq!(
            decode_utf16(b"a\x00\x00\xd8", false, "utf-16-le")
                .err()
                .unwrap()
                .message,
            "'utf-16-le' codec can't decode bytes in position 2-3: unexpected end of data"
        );
    }

    #[test]
    fn pathlib_strings() {
        if cfg!(windows) {
            assert_eq!(pathlib_normalise("./data/"), "data");
            assert_eq!(pathlib_normalise("C:/x//y/./z/"), "C:\\x\\y\\z");
            assert_eq!(pathlib_normalise("a/../b"), "a\\..\\b");
        } else {
            assert_eq!(pathlib_normalise("./data/"), "data");
            assert_eq!(pathlib_normalise("//x//y/"), "//x/y");
        }
        assert_eq!(pathlib_normalise(""), ".");
    }
}
