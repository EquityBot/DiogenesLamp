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
    if unbuffered {
        #[cfg(unix)]
        let handle = std::os::fd::AsFd::as_fd(&stdout)
            .try_clone_to_owned()
            .map(std::fs::File::from);
        #[cfg(windows)]
        let handle = std::os::windows::io::AsHandle::as_handle(&stdout)
            .try_clone_to_owned()
            .map(std::fs::File::from);
        if let Ok(file) = handle {
            return Box::new(file);
        }
    }
    Box::new(io::BufWriter::with_capacity(128 * 1024, stdout))
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

/// `(type(exc).__name__, str(exc))` for a failed write to a standard stream:
/// `OSError: [Errno 22] Invalid argument` for a closed pipe on Windows,
/// `BrokenPipeError: [Errno 32] Broken pipe` elsewhere.
fn stream_error(error: &io::Error) -> (&'static str, String) {
    let mut text = error.to_string();
    if let Some(at) = text.rfind(" (os error ") {
        text.truncate(at);
    }
    match error.raw_os_error() {
        Some(112) if cfg!(windows) => ("OSError", "[Errno 28] No space left on device".into()),
        Some(_) if cfg!(windows) => ("OSError", "[Errno 22] Invalid argument".into()),
        Some(32) => ("BrokenPipeError", format!("[Errno 32] {text}")),
        Some(code) => ("OSError", format!("[Errno {code}] {text}")),
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
