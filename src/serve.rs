//! Serve the viewer and the data directory on localhost, as Python's
//! `ThreadingHTTPServer` with `SimpleHTTPRequestHandler` (HTTP/1.0, one
//! request per connection) and DiogenesLamp's handler overrides.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

use crate::mimetable;
use crate::pyerr;
use crate::pystr;
use crate::pyurl;
use crate::viewer;

/// A name the scraper never writes, under the data directory: refused
/// requests are pointed at it so they 404 by not existing.
pub const REFUSED: &str = ".refused-traversal";
const SERVER_VERSION: &str = "SimpleHTTP/0.6 DiogenesLamp/0.1";
const CSP: &str = "default-src 'none'; img-src 'self' data:; script-src 'self'; style-src 'self'; connect-src 'self'; manifest-src 'self'; base-uri 'none'; form-action 'none'";

const ERROR_TEMPLATE: &str = "<!DOCTYPE HTML>
<html lang=\"en\">
    <head>
        <meta charset=\"utf-8\">
        <style type=\"text/css\">
            :root {
                color-scheme: light dark;
            }
        </style>
        <title>Error response</title>
    </head>
    <body>
        <h1>Error response</h1>
        <p>Error code: %(code)d</p>
        <p>Message: %(message)s.</p>
        <p>Error code explanation: %(code)s - %(explain)s.</p>
    </body>
</html>
";

fn status_text(code: u16) -> (&'static str, &'static str) {
    match code {
        200 => ("OK", "Request fulfilled, document follows"),
        301 => (
            "Moved Permanently",
            "Object moved permanently -- see URI list",
        ),
        304 => ("Not Modified", "Document has not changed since given time"),
        400 => ("Bad Request", "Bad request syntax or unsupported method"),
        404 => ("Not Found", "Nothing matches the given URI"),
        414 => ("URI Too Long", "URI is too long"),
        431 => (
            "Request Header Fields Too Large",
            "The server refused this request because the request header fields are too large",
        ),
        501 => ("Not Implemented", "Server does not support this operation"),
        505 => ("HTTP Version Not Supported", "Cannot fulfill request"),
        _ => ("???", "???"),
    }
}

/// Where viewer files come from: a directory, or the copies in the binary.
#[derive(Clone, Debug)]
pub enum Viewer {
    Dir(PathBuf),
    Embedded,
}

/// `email.utils.formatdate(timestamp, usegmt=True)`.
pub fn http_date(timestamp: i64) -> String {
    let days = timestamp.div_euclid(86_400);
    let secs = timestamp.rem_euclid(86_400);
    let date = crate::isodate::Date {
        ordinal: days + 719_163,
    };
    let (y, m, d) = date.ymd();
    let weekday = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]
        [(date.ordinal - 1).rem_euclid(7) as usize];
    let month = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ][(m - 1) as usize];
    format!(
        "{weekday}, {d:02} {month} {y:04} {:02}:{:02}:{:02} GMT",
        secs / 3600,
        secs % 3600 / 60,
        secs % 60
    )
}

/// `email.utils.parsedate_to_datetime`, kept to a UTC timestamp; None for
/// dates Python ignores or would not compare (non-UTC offsets).
fn parse_http_date(text: &str) -> Option<i64> {
    let mut words: Vec<&str> = text.split_whitespace().collect();
    if words
        .first()
        .is_some_and(|w| w.ends_with(',') || w.len() == 3 && w.chars().all(char::is_alphabetic))
    {
        words.remove(0);
    }
    if words.len() < 4 {
        return None;
    }
    let (day, month, year) = if words[0].chars().all(|c| c.is_ascii_digit()) {
        (words[0], words[1], words[2])
    } else {
        (words[1], words[0], words[2])
    };
    let month = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ]
    .iter()
    .position(|m| month.to_lowercase().starts_with(m))?
        + 1;
    let day: i64 = day.parse().ok()?;
    let mut year: i64 = year.parse().ok()?;
    if year < 100 {
        year += if year > 68 { 1900 } else { 2000 };
    }
    let time: Vec<i64> = words[3]
        .split(':')
        .map(|p| p.parse().unwrap_or(-1))
        .collect();
    let (h, mi, s) = match time.as_slice() {
        [h, m] => (*h, *m, 0),
        [h, m, s] => (*h, *m, *s),
        _ => return None,
    };
    if h < 0 || mi < 0 || s < 0 {
        return None;
    }
    match words.get(4).map(|z| z.to_uppercase()) {
        None => {}
        Some(z) if matches!(z.as_str(), "GMT" | "UT" | "UTC" | "Z" | "+0000" | "-0000") => {}
        Some(_) => return None,
    }
    let date = crate::isodate::Date::from_ymd(year, month as i64, day)?;
    Some((date.ordinal - 719_163) * 86_400 + h * 3600 + mi * 60 + s)
}

/// `SimpleHTTPRequestHandler.guess_type`.
fn guess_type(path: &str) -> String {
    fn splitext(path: &str) -> (String, String) {
        let sep = path.rfind(['/', '\\']).map_or(0, |i| i + 1);
        let name = &path[sep..];
        let lead = name.len() - name.trim_start_matches('.').len();
        match name[lead..].rfind('.') {
            Some(i) => (
                path[..sep + lead + i].to_string(),
                name[lead + i..].to_string(),
            ),
            None => (path.to_string(), String::new()),
        }
    }
    let (_, ext) = splitext(path);
    match ext.as_str() {
        ".gz" => return "application/gzip".into(),
        ".Z" => return "application/octet-stream".into(),
        ".bz2" => return "application/x-bzip2".into(),
        ".xz" => return "application/x-xz".into(),
        _ => {}
    }
    match ext.to_lowercase().as_str() {
        ".gz" => return "application/gzip".into(),
        ".bz2" => return "application/x-bzip2".into(),
        ".xz" => return "application/x-xz".into(),
        _ => {}
    }
    let path = if cfg!(windows) && path.as_bytes().get(1) == Some(&b':') {
        &path[2..]
    } else {
        path
    };
    let (mut base, mut ext) = splitext(path);
    while let Some((_, replacement)) = mimetable::SUFFIXES
        .iter()
        .find(|(k, _)| *k == ext.to_lowercase())
    {
        let (b, e) = splitext(&format!("{base}{replacement}"));
        base = b;
        ext = e;
    }
    if mimetable::ENCODINGS.contains(&ext.as_str()) {
        let (_, e) = splitext(&base);
        ext = e;
    }
    let ext = ext.to_lowercase();
    mimetable::TYPES
        .iter()
        .find(|(k, _)| *k == ext)
        .map_or_else(
            || "application/octet-stream".into(),
            |(_, v)| (*v).to_string(),
        )
}

/// `posixpath.normpath`.
fn posix_normpath(path: &str) -> String {
    if path.is_empty() {
        return ".".into();
    }
    let initial = if path.starts_with("//") && !path.starts_with("///") {
        2
    } else {
        usize::from(path.starts_with('/'))
    };
    let mut parts: Vec<&str> = Vec::new();
    for comp in path.split('/') {
        if comp.is_empty() || comp == "." {
            continue;
        }
        if comp != ".." || (initial == 0 && parts.is_empty()) || parts.last() == Some(&"..") {
            parts.push(comp);
        } else if !parts.is_empty() {
            parts.pop();
        }
    }
    let joined = format!("{}{}", "/".repeat(initial), parts.join("/"));
    if joined.is_empty() {
        ".".into()
    } else {
        joined
    }
}

/// The words `translate_path` keeps: no drive, separator, `.` or `..`.
fn kept_words(path: &str) -> (Vec<String>, bool) {
    let path = path.split('#').next().unwrap_or("");
    let path = path.split('?').next().unwrap_or("");
    let path = pyurl::unquote(path);
    let trailing = path.ends_with('/');
    let normal = posix_normpath(&path);
    let words = normal
        .split('/')
        .filter(|w| !w.is_empty())
        .filter(|w| {
            let has_dir = if cfg!(windows) {
                w.contains('\\') || w.contains(':')
            } else {
                false
            };
            !has_dir && *w != "." && *w != ".."
        })
        .map(str::to_string)
        .collect();
    (words, trailing)
}

/// A resolved request target.
#[derive(Clone, Debug)]
enum Resolved {
    Disk(PathBuf, bool),
    Embedded(Vec<String>, bool),
}

impl Resolved {
    fn display(&self) -> String {
        match self {
            Resolved::Disk(path, trailing) => {
                let mut s = pyerr::path_str(path);
                if *trailing {
                    s.push('/');
                }
                s
            }
            Resolved::Embedded(words, trailing) => {
                let mut s = words.join("/");
                if *trailing {
                    s.push('/');
                }
                s
            }
        }
    }
}

struct Handler<'a> {
    viewer: &'a Viewer,
    data_dir: &'a Path,
    stream: &'a mut TcpStream,
    path: String,
    command: String,
    request_version: String,
    headers: Vec<(String, String)>,
    status: u16,
    shard_etag: Option<String>,
    buffer: Vec<u8>,
}

impl Handler<'_> {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    fn send_response(&mut self, code: u16, message: Option<&str>) {
        self.status = code;
        if self.request_version != "HTTP/0.9" {
            let message = message.unwrap_or(status_text(code).0);
            self.buffer
                .extend(format!("HTTP/1.0 {code} {message}\r\n").bytes().map(latin1));
        }
        self.send_header("Server", SERVER_VERSION);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        self.send_header("Date", &http_date(now));
    }

    fn send_header(&mut self, key: &str, value: &str) {
        if self.request_version != "HTTP/0.9" {
            self.buffer.extend(
                format!("{key}: {value}\r\n")
                    .chars()
                    .map(|c| c as u32 as u8),
            );
        }
    }

    fn end_headers(&mut self) {
        let cache_ok = matches!(self.status, 200 | 304);
        if self.path.starts_with("/data/thumbs/") && cache_ok {
            self.send_header("Cache-Control", "public, max-age=86400");
        } else if let Some(etag) = self.shard_etag.clone().filter(|_| cache_ok) {
            self.send_header("Cache-Control", "no-cache");
            self.send_header("ETag", &etag);
        } else {
            self.send_header("Cache-Control", "no-store");
        }
        self.send_header("Content-Security-Policy", CSP);
        self.send_header("X-Content-Type-Options", "nosniff");
        if self.request_version != "HTTP/0.9" {
            self.buffer.extend_from_slice(b"\r\n");
            let data = std::mem::take(&mut self.buffer);
            let _ = self.stream.write_all(&data);
        }
    }

    fn send_error(&mut self, code: u16, message: Option<&str>, explain: Option<&str>) {
        let (short, long) = status_text(code);
        let message = message.unwrap_or(short).to_string();
        let explain = explain.unwrap_or(long).to_string();
        self.send_response(code, Some(&message));
        self.send_header("Connection", "close");
        let mut body = None;
        if code >= 200 && !matches!(code, 204 | 205 | 304) {
            let escape = |s: &str| {
                s.replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;")
            };
            let content = ERROR_TEMPLATE
                .replace("%(code)d", &code.to_string())
                .replace("%(code)s", &code.to_string())
                .replace("%(message)s", &escape(&message))
                .replace("%(explain)s", &escape(&explain));
            self.send_header("Content-Type", "text/html;charset=utf-8");
            self.send_header("Content-Length", &content.len().to_string());
            body = Some(content.into_bytes());
        }
        self.end_headers();
        if self.command != "HEAD"
            && let Some(body) = body
        {
            let _ = self.stream.write_all(&body);
        }
    }

    fn translate_path(&self, path: &str) -> Resolved {
        let clean = pyurl::unquote(
            path.split('?')
                .next()
                .unwrap_or("")
                .split('#')
                .next()
                .unwrap_or(""),
        );
        let refused = || Resolved::Disk(self.data_dir.join(REFUSED), false);
        if clean.contains('\0') {
            return refused();
        }
        if let Some(relative) = clean.strip_prefix("/data/") {
            let candidate = pyerr::resolve(&self.data_dir.join(relative));
            if candidate.starts_with(pyerr::resolve(self.data_dir)) {
                return Resolved::Disk(candidate, false);
            }
            return refused();
        }
        let (words, trailing) = kept_words(path);
        match self.viewer {
            Viewer::Dir(dir) => {
                let mut target = dir.clone();
                for word in &words {
                    target.push(word);
                }
                Resolved::Disk(target, trailing)
            }
            Viewer::Embedded => Resolved::Embedded(words, trailing),
        }
    }

    fn write_body(&mut self, body: &[u8]) {
        if self.command != "HEAD" {
            let _ = self.stream.write_all(body);
        }
    }

    /// DiogenesLamp's override: shards are served with a content ETag.
    fn send_head(&mut self) {
        self.shard_etag = None;
        let request_path = pyurl::unquote(self.path.split('?').next().unwrap_or(""));
        if request_path.starts_with("/data/catalog/") && request_path.ends_with(".json") {
            let path = self.path.clone();
            if let Resolved::Disk(target, _) = self.translate_path(&path) {
                let shard = pyerr::resolve(&target);
                if shard.starts_with(pyerr::resolve(&self.data_dir.join("catalog")))
                    && let Ok(body) = std::fs::read(&shard)
                {
                    let digest = Sha256::digest(&body);
                    let etag = format!(
                        "\"{}\"",
                        digest
                            .iter()
                            .map(|b| format!("{b:02x}"))
                            .collect::<String>()
                    );
                    self.shard_etag = Some(etag.clone());
                    let validators = self.header("If-None-Match").unwrap_or("").to_string();
                    let matched = validators.split(',').any(|v| {
                        let v = pystr::strip(v);
                        let v = v.strip_prefix("W/").unwrap_or(v);
                        v == "*" || v == etag
                    });
                    if matched {
                        self.send_response(304, None);
                        self.end_headers();
                        return;
                    }
                    self.send_response(200, None);
                    self.send_header("Content-Type", "application/json");
                    self.send_header("Content-Length", &body.len().to_string());
                    self.end_headers();
                    self.write_body(&body);
                    return;
                }
            }
        }
        self.simple_send_head();
    }

    /// `SimpleHTTPRequestHandler.send_head`.
    fn simple_send_head(&mut self) {
        let path = self.path.clone();
        let resolved = self.translate_path(&path);
        let is_dir = match &resolved {
            Resolved::Disk(p, _) => p.is_dir(),
            Resolved::Embedded(words, _) => words.is_empty(),
        };
        let mut resolved = resolved;
        if is_dir {
            let Ok(parts) = pyurl::urlsplit(&self.path) else {
                return;
            };
            if !(parts.path.ends_with('/')
                || parts.path.ends_with("%2f")
                || parts.path.ends_with("%2F"))
            {
                self.send_response(301, None);
                let location = pyurl::urlunsplit(
                    &parts.scheme,
                    parts.netloc_str(),
                    &format!("{}/", parts.path),
                    parts.query_str(),
                    parts.fragment.as_deref().unwrap_or(""),
                );
                self.send_header("Location", &location);
                self.send_header("Content-Length", "0");
                self.end_headers();
                return;
            }
            let index = match &resolved {
                Resolved::Disk(p, _) => ["index.html", "index.htm"]
                    .iter()
                    .map(|n| p.join(n))
                    .find(|p| p.is_file())
                    .map(|p| Resolved::Disk(p, false)),
                Resolved::Embedded(..) => viewer::get("index.html")
                    .map(|_| Resolved::Embedded(vec!["index.html".into()], false)),
            };
            match index {
                Some(index) => resolved = index,
                None => {
                    self.send_error(404, Some("No permission to list directory"), None);
                    return;
                }
            }
        }
        let display = resolved.display();
        let ctype = guess_type(&display);
        if display.ends_with('/') {
            self.send_error(404, Some("File not found"), None);
            return;
        }
        match resolved {
            Resolved::Embedded(words, _) => {
                let data = (words.len() == 1).then(|| viewer::get(&words[0])).flatten();
                let Some(data) = data else {
                    self.send_error(404, Some("File not found"), None);
                    return;
                };
                self.send_response(200, None);
                self.send_header("Content-type", &ctype);
                self.send_header("Content-Length", &data.len().to_string());
                self.end_headers();
                self.write_body(data);
            }
            Resolved::Disk(path, _) => {
                let opened = if path.is_dir() {
                    None
                } else {
                    std::fs::File::open(&path).ok()
                };
                let Some(mut file) = opened else {
                    self.send_error(404, Some("File not found"), None);
                    return;
                };
                let meta = file.metadata().ok();
                let mtime = meta
                    .as_ref()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_secs() as i64);
                if self.header("If-Modified-Since").is_some()
                    && self.header("If-None-Match").is_none()
                    && let Some(ims) = self.header("If-Modified-Since").and_then(parse_http_date)
                    && mtime <= ims
                {
                    self.send_response(304, None);
                    self.end_headers();
                    return;
                }
                let mut data = Vec::new();
                if file.read_to_end(&mut data).is_err() {
                    return;
                }
                self.send_response(200, None);
                self.send_header("Content-type", &ctype);
                self.send_header(
                    "Content-Length",
                    &meta.map_or(data.len() as u64, |m| m.len()).to_string(),
                );
                self.send_header("Last-Modified", &http_date(mtime));
                self.end_headers();
                self.write_body(&data);
            }
        }
    }
}

fn latin1(b: u8) -> u8 {
    b
}

fn handle(stream: &mut TcpStream, viewer: &Viewer, data_dir: &Path) {
    let Ok(clone) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(clone);
    let mut raw = Vec::new();
    if reader
        .by_ref()
        .take(65537)
        .read_until(b'\n', &mut raw)
        .is_err()
    {
        return;
    }
    let mut handler = Handler {
        viewer,
        data_dir,
        stream,
        path: String::new(),
        command: String::new(),
        request_version: "HTTP/0.9".into(),
        headers: Vec::new(),
        status: 0,
        shard_etag: None,
        buffer: Vec::new(),
    };
    // Python's end_headers override reads self.path, which is unset until
    // the request line parses: those error replies die before a byte is
    // sent, and the connection just closes.
    if raw.len() > 65536 {
        return;
    }
    if raw.is_empty() {
        return;
    }
    let line: String = raw.iter().map(|&b| b as char).collect();
    let line = line.trim_end_matches(['\r', '\n']);
    let words: Vec<&str> = line
        .split(pystr::is_space)
        .filter(|w| !w.is_empty())
        .collect();
    if words.is_empty() {
        return;
    }
    if words.len() >= 3 {
        let version = words[words.len() - 1];
        let parsed = version.strip_prefix("HTTP/").and_then(|v| {
            let parts: Vec<&str> = v.split('.').collect();
            if parts.len() != 2
                || parts
                    .iter()
                    .any(|p| !pystr::all_digits(p) || p.chars().count() > 10)
            {
                return None;
            }
            Some((
                pystr::parse_int(parts[0])?,
                pystr::parse_int(parts[1])?,
                v.to_string(),
            ))
        });
        let Some((major, _minor, _base)) = parsed else {
            return;
        };
        handler.request_version = version.to_string();
        if major >= 2 {
            return;
        }
    }
    if !(2..=3).contains(&words.len()) {
        return;
    }
    let (command, path) = (words[0].to_string(), words[1].to_string());
    if words.len() == 2 && command != "GET" {
        return;
    }
    handler.command = command;
    handler.path = if path.starts_with("//") {
        format!("/{}", path.trim_start_matches('/'))
    } else {
        path
    };
    if words.len() == 3 {
        let mut count = 0;
        loop {
            let mut header_line = Vec::new();
            if reader
                .by_ref()
                .take(65537)
                .read_until(b'\n', &mut header_line)
                .is_err()
            {
                return;
            }
            if header_line.len() > 65536 {
                handler.send_error(
                    431,
                    Some("Line too long"),
                    Some("got more than 65536 bytes when reading header line"),
                );
                return;
            }
            if header_line.is_empty() || header_line == b"\r\n" || header_line == b"\n" {
                break;
            }
            count += 1;
            if count > 100 {
                handler.send_error(
                    431,
                    Some("Too many headers"),
                    Some("got more than 100 headers"),
                );
                return;
            }
            let text: String = header_line.iter().map(|&b| b as char).collect();
            if let Some((name, value)) = text.split_once(':') {
                handler
                    .headers
                    .push((name.trim().to_string(), value.trim().to_string()));
            }
        }
    }
    match handler.command.as_str() {
        "GET" | "HEAD" => handler.send_head(),
        other => {
            let message = format!("Unsupported method ({})", pystr::repr(other));
            handler.send_error(501, Some(&message), None);
        }
    }
    let _ = handler.stream.flush();
}

pub struct Server {
    listener: TcpListener,
    viewer: Viewer,
    data_dir: PathBuf,
    shutdown: Arc<AtomicBool>,
}

/// Bind 127.0.0.1:port (0 picks a free port).
pub fn make_server(viewer: Viewer, data_dir: &Path, port: u16) -> std::io::Result<Server> {
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    listener.set_nonblocking(true)?;
    Ok(Server {
        listener,
        viewer,
        data_dir: data_dir.to_path_buf(),
        shutdown: Arc::new(AtomicBool::new(false)),
    })
}

impl Server {
    pub fn port(&self) -> u16 {
        self.listener.local_addr().map_or(0, |a| a.port())
    }

    pub fn shutdown_handle(&self) -> Arc<AtomicBool> {
        self.shutdown.clone()
    }

    /// Serve until the shutdown flag is set; each request on its own thread.
    pub fn serve_forever(&self) {
        while !self.shutdown.load(Ordering::SeqCst) {
            match self.listener.accept() {
                Ok((mut stream, _)) => {
                    let _ = stream.set_nonblocking(false);
                    let viewer = self.viewer.clone();
                    let data_dir = self.data_dir.clone();
                    std::thread::spawn(move || {
                        handle(&mut stream, &viewer, &data_dir);
                        let _ = stream.shutdown(std::net::Shutdown::Both);
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(_) => std::thread::sleep(Duration::from_millis(10)),
            }
        }
    }
}

/// Open the default browser at `url`, as `webbrowser.open` does.
fn open_browser(url: &str) {
    #[cfg(windows)]
    let result = std::process::Command::new("rundll32")
        .args(["url.dll,FileProtocolHandler", url])
        .spawn();
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg(url).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let result = std::process::Command::new("xdg-open").arg(url).spawn();
    let _ = result;
}

/// `lamp serve`: print the URL, optionally open a browser, serve until Ctrl+C.
pub fn serve(server: &Server, open: bool, stop: Arc<AtomicBool>) {
    let url = format!("http://127.0.0.1:{}/", server.port());
    crate::out::out(&format!("serving on {url} (Ctrl+C to stop)"))
        .unwrap_or_else(|e| crate::out::uncaught(&e));
    if open {
        open_browser(&url);
    }
    let flag = server.shutdown_handle();
    std::thread::spawn(move || {
        while !stop.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(50));
        }
        flag.store(true, Ordering::SeqCst);
    });
    server.serve_forever();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_and_types() {
        assert_eq!(http_date(0), "Thu, 01 Jan 1970 00:00:00 GMT");
        assert_eq!(
            parse_http_date("Fri, 01 Jan 2100 00:00:00 GMT"),
            Some(4_102_444_800)
        );
        assert_eq!(guess_type("x/app.js"), "text/javascript");
        assert_eq!(guess_type("x/a.json"), "application/json");
        assert_eq!(guess_type("x/1.jpg"), "image/jpeg");
        assert_eq!(guess_type("x/noext"), "application/octet-stream");
        assert_eq!(posix_normpath("/data/../x"), "/x");
        assert_eq!(posix_normpath("/../x"), "/x");
    }
}
