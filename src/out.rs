//! Console output as Python's `print` writes it: UTF-8, and `\n` becomes
//! `\r\n` on Windows (Python's text-mode stdio translates newlines there).
//!
//! The streams also fail as the Python release's did. stdout is line-buffered
//! on a terminal, unbuffered with `PYTHONUNBUFFERED`, and otherwise held in a
//! 128 KiB buffer (CPython 3.15's `io.DEFAULT_BUFFER_SIZE`) until it fills or
//! the process exits, so a closed stdout usually fails only at exit. A failed
//! write to stderr raised at once and nothing in lamp caught it.

use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex, MutexGuard, PoisonError};

use crate::errors::Error;

static UNBUFFERED: LazyLock<bool> = LazyLock::new(python_unbuffered);
static STDOUT: LazyLock<Mutex<Box<dyn Write + Send>>> =
    LazyLock::new(|| Mutex::new(python_stdout(*UNBUFFERED)));
/// A write to stderr failed. Buffered, its bytes stayed pending in Python's
/// stderr and failed again at exit.
static STDERR_FAILED: AtomicBool = AtomicBool::new(false);

fn translate(text: &str) -> String {
    crate::pyerr::native_newlines(text)
}

/// `PYTHONUNBUFFERED` as CPython reads it: set and not an integer equal to 0.
fn python_unbuffered() -> bool {
    std::env::var("PYTHONUNBUFFERED").is_ok_and(|value| {
        !value.is_empty()
            && value
                .trim_start_matches([' ', '\t', '\n', '\x0b', '\x0c', '\r'])
                .parse::<i32>()
                != Ok(0)
    })
}

fn python_stdout(unbuffered: bool) -> Box<dyn Write + Send> {
    use std::io::IsTerminal;
    let stdout = io::stdout();
    if stdout.is_terminal() {
        return Box::new(io::LineWriter::new(stdout));
    }
    // A file on a duplicate of the descriptor, because std's stdout reports a
    // write to a bad descriptor (EBADF on Unix) as success. Without one there
    // is no stdout to duplicate, and Python's sys.stdout was None: std's
    // stdout then discards writes as Python's print did.
    #[cfg(unix)]
    let handle = std::os::fd::AsFd::as_fd(&stdout)
        .try_clone_to_owned()
        .map(std::fs::File::from);
    // A missing Windows handle is null, which duplicates without error.
    #[cfg(windows)]
    let handle = if std::os::windows::io::AsRawHandle::as_raw_handle(&stdout).is_null() {
        Err(io::ErrorKind::NotFound.into())
    } else {
        std::os::windows::io::AsHandle::as_handle(&stdout)
            .try_clone_to_owned()
            .map(std::fs::File::from)
    };
    match handle {
        Ok(file) if unbuffered => Box::new(file),
        Ok(file) => Box::new(io::BufWriter::with_capacity(128 * 1024, file)),
        Err(_) => Box::new(io::BufWriter::with_capacity(128 * 1024, stdout)),
    }
}

fn stdout() -> MutexGuard<'static, Box<dyn Write + Send>> {
    STDOUT.lock().unwrap_or_else(PoisonError::into_inner)
}

fn write_stderr(text: &str) -> io::Result<()> {
    io::stderr()
        .lock()
        .write_all(translate(text).as_bytes())
        .inspect_err(|_| STDERR_FAILED.store(true, Ordering::SeqCst))
}

/// The errno the C runtime's `write()` hands Python for a Windows error: a
/// handle without write access is EBADF, and the rest go through _dosmaperr,
/// so a closed pipe (ERROR_NO_DATA) is EINVAL.
#[cfg(windows)]
fn crt_write_errno(code: i32) -> i32 {
    if code == 5 {
        9
    } else {
        crate::pyerr::crt_errno(code)
    }
}

/// The OSError subclass Python raises for an errno.
fn errno_class(errno: i32) -> &'static str {
    match errno {
        1 | 13 => "PermissionError",
        2 => "FileNotFoundError",
        17 => "FileExistsError",
        32 => "BrokenPipeError",
        _ => "OSError",
    }
}

/// `(type(exc).__name__, str(exc))` for a failed write to a standard stream:
/// `OSError: [Errno 22] Invalid argument` for a closed pipe on Windows,
/// `BrokenPipeError: [Errno 32] Broken pipe` elsewhere.
fn stream_error(error: &io::Error) -> (&'static str, String) {
    #[cfg(windows)]
    if let Some(code) = error.raw_os_error() {
        let errno = crt_write_errno(code);
        let text = crate::pyerr::errno_text(errno);
        return (errno_class(errno), format!("[Errno {errno}] {text}"));
    }
    let mut text = error.to_string();
    if let Some(at) = text.rfind(" (os error ") {
        text.truncate(at);
    }
    match error.raw_os_error() {
        Some(code) => (errno_class(code), format!("[Errno {code}] {text}")),
        None => ("OSError", text),
    }
}

/// A failed `print` as the OSError a handler catches.
pub fn os_error(error: io::Error) -> Error {
    Error::Os(stream_error(&error).1)
}

/// `print(text)`.
pub fn out(text: &str) -> io::Result<()> {
    stdout().write_all(translate(&format!("{text}\n")).as_bytes())
}

/// `print(text, flush=True)`.
pub fn out_flush(text: &str) -> io::Result<()> {
    let mut stdout = stdout();
    stdout.write_all(translate(&format!("{text}\n")).as_bytes())?;
    stdout.flush()
}

/// Raw stdout text with no newline added (argparse, which ignores write errors).
pub fn out_raw(text: &str) {
    let _ = stdout().write_all(translate(text).as_bytes());
}

/// `print(text, file=sys.stderr)`. Its OSError went uncaught, and the
/// traceback could not be written either.
pub fn err(text: &str) {
    if write_stderr(&format!("{text}\n")).is_err() {
        std::process::exit(shutdown(1));
    }
}

/// `print(text, file=sys.stderr)` from a scrape worker. Python's uncaught
/// OSError ended only that thread; this carries on.
pub fn err_from_worker(text: &str) {
    let _ = write_stderr(&format!("{text}\n"));
}

/// Raw stderr text with no newline added (argparse, which ignores write errors).
pub fn err_raw(text: &str) {
    let _ = write_stderr(text);
}

/// An OSError from `print` that nothing caught: its traceback, then exit.
pub fn uncaught(error: &io::Error) -> ! {
    let (kind, message) = stream_error(error);
    let _ = write_stderr(&format!("{kind}: {message}\n"));
    std::process::exit(shutdown(1))
}

/// Interpreter shutdown: flush stdout, then stderr. Output still pending in
/// either fails there with exit status 120.
pub fn shutdown(code: i32) -> i32 {
    let mut code = code;
    if let Err(error) = stdout().flush() {
        let (kind, message) = stream_error(&error);
        let _ = write_stderr(&format!(
            "Exception ignored while flushing sys.stdout:\n{kind}: {message}\n"
        ));
        code = 120;
    }
    if STDERR_FAILED.load(Ordering::SeqCst) && !*UNBUFFERED {
        code = 120;
    }
    code
}

#[cfg(all(test, windows))]
mod tests {
    use super::stream_error;
    use std::io;

    /// Windows write errors as CPython 3.15 reports them through the C runtime.
    #[test]
    fn windows_write_errors_read_as_python_reports_them() {
        for (code, kind, message) in [
            (232, "OSError", "[Errno 22] Invalid argument"),
            (5, "OSError", "[Errno 9] Bad file descriptor"),
            (6, "OSError", "[Errno 9] Bad file descriptor"),
            (109, "BrokenPipeError", "[Errno 32] Broken pipe"),
            (112, "OSError", "[Errno 28] No space left on device"),
            (29, "PermissionError", "[Errno 13] Permission denied"),
            (31, "PermissionError", "[Errno 13] Permission denied"),
            (114, "OSError", "[Errno 9] Bad file descriptor"),
            (130, "OSError", "[Errno 9] Bad file descriptor"),
            (267, "OSError", "[Errno 22] Invalid argument"),
            (8, "OSError", "[Errno 12] Not enough space"),
        ] {
            let error = io::Error::from_raw_os_error(code);
            assert_eq!(stream_error(&error), (kind, message.to_string()), "{code}");
        }
    }
}
