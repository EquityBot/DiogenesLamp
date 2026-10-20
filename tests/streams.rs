//! A standard stream that cannot be written fails as it did for the Python
//! release, measured against it on a closed pipe: stdout still pending at exit
//! is exit status 120, a maintenance command's handler reports a failed print,
//! an uncaught one exits 1, a diagnostic that cannot be written loses the
//! original code, and argparse ignores its own write errors.
mod common;

use std::path::Path;
use std::process::{Command, Stdio};

/// The OSError Python raised for a write to a pipe with no reader.
const CLOSED_PIPE: &str = if cfg!(windows) {
    "OSError: [Errno 22] Invalid argument"
} else {
    "BrokenPipeError: [Errno 32] Broken pipe"
};

fn closed_pipe() -> Stdio {
    let (reader, writer) = std::io::pipe().unwrap();
    drop(reader);
    writer.into()
}

/// Exit code and the text on the stream that still works.
fn run(cwd: &Path, args: &[&str], broken_stdout: bool, unbuffered: bool) -> (i32, String) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_lamp"));
    command
        .args(args)
        .current_dir(cwd)
        .env("COLUMNS", "80")
        .env_remove("PYTHONUNBUFFERED");
    if unbuffered {
        command.env("PYTHONUNBUFFERED", "1");
    }
    let output = if broken_stdout {
        command.stdout(closed_pipe()).stderr(Stdio::piped())
    } else {
        command.stdout(Stdio::piped()).stderr(closed_pipe())
    }
    .output()
    .unwrap();
    let other = if broken_stdout {
        output.stderr
    } else {
        output.stdout
    };
    (
        output.status.code().unwrap(),
        String::from_utf8(other).unwrap().replace("\r\n", "\n"),
    )
}

/// A catalogue directory, a dump to import and a seller whose fetch is
/// refused before any request is sent.
fn workspace() -> tempfile::TempDir {
    let dir = common::tmp();
    common::write(&dir.path().join("data").join("readme"), "");
    common::write(
        &dir.path().join("links.txt"),
        "https://exampleseller.x.yupoo.com/albums\n",
    );
    common::write(
        &dir.path().join("offline.toml"),
        "[yupoo.bad]\nurl = \"https://127.0.0.1:9\"\nname = \"bad\"\n",
    );
    dir
}

#[test]
fn pending_stdout_fails_at_exit_with_status_120() {
    let dir = workspace();
    let flush = format!("Exception ignored while flushing sys.stdout:\n{CLOSED_PIPE}\n");
    for args in [&["--help"][..], &["doctor"], &["import", "links.txt"]] {
        assert_eq!(
            run(dir.path(), args, true, false),
            (120, flush.clone()),
            "{args:?}"
        );
    }
}

#[test]
fn unbuffered_stdout_fails_at_the_print() {
    let dir = workspace();
    let message = CLOSED_PIPE.split_once(": ").unwrap().1;
    assert_eq!(run(dir.path(), &["--help"], true, true), (0, String::new()));
    assert_eq!(
        run(dir.path(), &["doctor"], true, true),
        (1, format!("error: {message}\n"))
    );
    assert_eq!(
        run(dir.path(), &["import", "links.txt"], true, true),
        (1, format!("{CLOSED_PIPE}\n"))
    );
}

#[test]
fn a_scrape_report_line_fails_at_once() {
    let dir = workspace();
    let scrape = ["--sources", "offline.toml", "scrape"];
    let (code, stderr) = run(dir.path(), &scrape, true, false);
    assert_eq!(code, 120);
    assert_eq!(
        stderr,
        format!("{CLOSED_PIPE}\nException ignored while flushing sys.stdout:\n{CLOSED_PIPE}\n")
    );
    assert_eq!(
        run(dir.path(), &scrape, true, true),
        (1, format!("{CLOSED_PIPE}\n"))
    );
    // The run stopped before the index was written.
    assert!(!dir.path().join("data").join("index.json").exists());
}

#[test]
fn an_unwritable_diagnostic_loses_the_original_code() {
    let dir = workspace();
    let missing = ["import", "missing.txt"];
    assert_eq!(
        run(dir.path(), &missing, false, false),
        (120, String::new())
    );
    assert_eq!(run(dir.path(), &missing, false, true), (1, String::new()));
    assert_eq!(
        run(dir.path(), &["--bogus"], false, false),
        (120, String::new())
    );
    assert_eq!(
        run(dir.path(), &["--bogus"], false, true),
        (2, String::new())
    );
}
