//! Console output as Python's `print` writes it: UTF-8, and `\n` becomes
//! `\r\n` on Windows (Python's text-mode stdio translates newlines there).

use std::io::Write;

fn translate(text: &str) -> String {
    crate::pyerr::native_newlines(text)
}

/// `print(text)`.
pub fn out(text: &str) {
    let mut stdout = std::io::stdout().lock();
    let _ = stdout.write_all(translate(&format!("{text}\n")).as_bytes());
    let _ = stdout.flush();
}

/// `print(text, file=sys.stderr)`.
pub fn err(text: &str) {
    let mut stderr = std::io::stderr().lock();
    let _ = stderr.write_all(translate(&format!("{text}\n")).as_bytes());
    let _ = stderr.flush();
}

/// Raw stderr text with no newline added (argparse writes its own).
pub fn err_raw(text: &str) {
    let mut stderr = std::io::stderr().lock();
    let _ = stderr.write_all(translate(text).as_bytes());
    let _ = stderr.flush();
}

/// Raw stdout text with no newline added.
pub fn out_raw(text: &str) {
    let mut stdout = std::io::stdout().lock();
    let _ = stdout.write_all(translate(text).as_bytes());
    let _ = stdout.flush();
}
