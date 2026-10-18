//! Ported from tests/test_cli.py and tests/test_smoke.py.
mod common;

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use common::{lamp, read, write};
use lamp::cli::report_line;
use lamp::scrape::SellerSummary;

#[test]
fn package_version() {
    assert_eq!(env!("CARGO_PKG_VERSION"), "0.1.0");
}

#[test]
fn import_creates_sources() {
    let dir = common::tmp();
    write(
        &dir.path().join("links.csv"),
        "1\thttps://sellerx.x.yupoo.com/\n2\thttps://weidian.com/?userid=7\n3\tN/A\n",
    );
    let sources = dir.path().join("sources.toml");
    let (code, out, _) = lamp(
        &[
            "--sources",
            &sources.to_string_lossy(),
            "import",
            &dir.path().join("links.csv").to_string_lossy(),
        ],
        None,
    );
    assert_eq!(code, 0);
    let text = read(&sources);
    assert!(text.contains("[yupoo.sellerx]") && text.contains("[shops.weidian-7]"));
    assert!(out.contains("3 rows") && out.contains("1 invalid"));
    assert!(out.contains("yupoo sellers: 1 new, 0 known"));
    assert!(out.contains("shops: 1 new, 0 known"));
}

#[test]
fn import_reports_missing_and_undecodable_dumps() {
    let dir = common::tmp();
    let s = dir.path().join("s.toml").to_string_lossy().into_owned();
    let (code, _, err) = lamp(
        &[
            "--sources",
            &s,
            "import",
            &dir.path().join("nope.csv").to_string_lossy(),
        ],
        None,
    );
    assert_eq!(code, 1);
    assert!(err.contains("not found"));
    let dump = dir.path().join("dump.csv");
    write(&dump, b"https://ok.x.yupoo.com/\n\x81\x9d not utf-8");
    let (code, _, err) = lamp(&["--sources", &s, "import", &dump.to_string_lossy()], None);
    assert_eq!(code, 1);
    assert!(
        err.contains("could not read")
            && err.contains("utf-8")
            && err.contains("re-save it as UTF-8")
    );
    assert!(!dir.path().join("s.toml").exists());
}

#[test]
fn import_accepts_utf16_excel_exports() {
    let dir = common::tmp();
    let mut blob = vec![0xff, 0xfe];
    for unit in "1\thttps://sellerx.x.yupoo.com/\n".encode_utf16() {
        blob.extend_from_slice(&unit.to_le_bytes());
    }
    let dump = dir.path().join("dump.csv");
    write(&dump, blob);
    let s = dir.path().join("s.toml");
    let (code, _, _) = lamp(
        &[
            "--sources",
            &s.to_string_lossy(),
            "import",
            &dump.to_string_lossy(),
        ],
        None,
    );
    assert_eq!(code, 0);
    assert!(read(&s).contains("[yupoo.sellerx]"));
}

fn sources_file(dir: &std::path::Path) -> String {
    let path = dir.join("sources.toml");
    write(
        &path,
        "[yupoo.sellerx]\nurl = \"https://sellerx.x.yupoo.com\"\nname = \"sellerx\"\n",
    );
    path.to_string_lossy().into_owned()
}

#[test]
fn scrape_requires_sources_and_a_known_seller() {
    let dir = common::tmp();
    let data = dir.path().join("d").to_string_lossy().into_owned();
    let (code, _, err) = lamp(
        &[
            "--sources",
            &dir.path().join("none.toml").to_string_lossy(),
            "--data",
            &data,
            "scrape",
        ],
        None,
    );
    assert_eq!(code, 1);
    assert!(err.contains("no sources"));
    let (code, _, err) = lamp(
        &[
            "--sources",
            &sources_file(dir.path()),
            "--data",
            &data,
            "scrape",
            "--seller",
            "nosuchseller",
        ],
        None,
    );
    assert_eq!(code, 1);
    assert!(err.contains("unknown seller"));
}

#[test]
fn scrape_report_lines_name_each_diagnostic() {
    let summary = SellerSummary {
        scraped: 4,
        skipped: 2,
        failed: 1,
        failed_ids: vec!["9".into()],
        thumbs_blocked: true,
        dated: 0,
        skipped_old: 7,
        thumbs_refetched: 3,
        ..SellerSummary::new("sellerx")
    };
    let line = report_line(&summary, 1, 1, false);
    assert!(line.starts_with("[1/1] sellerx: 4 scraped, 2 skipped, 1 failed"));
    for part in [
        "7 too old",
        "3 covers fetched again",
        "failed albums: 9",
        "thumbs blocked",
        "no publish dates found",
    ] {
        assert!(line.contains(part), "{part}");
    }
    let errored = SellerSummary {
        error: "blocked: HTTP 429".into(),
        ..SellerSummary::new("sellerx")
    };
    assert!(report_line(&errored, 1, 1, false).contains("ERROR: blocked: HTTP 429"));
    let many = SellerSummary {
        failed_ids: (1..=7).map(|i| i.to_string()).collect(),
        ..SellerSummary::new("s")
    };
    assert!(report_line(&many, 2, 3, true).contains("(failed albums: 1, 2, 3, 4, 5, ...)"));
}

#[test]
fn serve_warns_when_there_is_no_index_and_needs_no_source_tree() {
    let dir = common::tmp();
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
        .to_string();
    let mut child = Command::new(env!("CARGO_BIN_EXE_lamp"))
        .args([
            "--data",
            &dir.path().to_string_lossy(),
            "serve",
            "--no-browser",
            "--port",
            &port,
        ])
        .current_dir(dir.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let started = Instant::now();
    let reply = loop {
        if let Ok(mut stream) =
            std::net::TcpStream::connect(("127.0.0.1", port.parse::<u16>().unwrap()))
        {
            use std::io::Write;
            stream.write_all(b"GET / HTTP/1.0\r\n\r\n").unwrap();
            let mut data = Vec::new();
            stream.read_to_end(&mut data).unwrap();
            break String::from_utf8_lossy(&data).into_owned();
        }
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "server did not start"
        );
        std::thread::sleep(Duration::from_millis(50));
    };
    child.kill().unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(reply.starts_with("HTTP/1.0 200"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("no index.json"));
    assert!(String::from_utf8_lossy(&output.stdout).contains("serving on http://127.0.0.1:"));
}

#[test]
fn nonsense_numeric_args_are_usage_errors() {
    for args in [
        &["scrape", "--workers", "0"][..],
        &["scrape", "--rps", "0"],
        &["scrape", "--rps", "-4"],
        &["scrape", "--rps", "nan"],
        &["scrape", "--rps", "inf"],
        &["serve", "--port", "0"],
        &["serve", "--port", "99999"],
    ] {
        let (code, out, err) = lamp(args, None);
        assert_eq!(code, 2, "{args:?}");
        assert!(out.is_empty() && err.contains("error"));
    }
}

#[test]
fn help_and_usage_follow_argparse() {
    let (code, out, _) = lamp(&["-h"], None);
    assert_eq!(code, 0);
    assert!(out.starts_with("usage: lamp [-h] [--sources SOURCES] [--data DATA]\n"));
    assert!(out.contains("  -h, --help            show this help message and exit\n"));
    let (code, _, err) = lamp(&["bogus"], None);
    assert_eq!(code, 2);
    assert!(err.contains(
        "lamp: error: argument command: invalid choice: 'bogus' (choose from 'import', 'scrape'"
    ));
    let (code, _, err) = lamp(&["scrape", "--re", "1"], None);
    assert_eq!(code, 2);
    assert!(err.ends_with("lamp scrape: error: ambiguous option: --re could match --refresh-details-days, --refresh-limit, --retry-failed\n"));
}
