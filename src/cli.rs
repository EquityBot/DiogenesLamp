//! lamp command line interface.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::argparse::{Parser, Value};
use crate::errors::Error;
use crate::ingest::{load_sources, merge_sources, parse_dump, write_sources};
use crate::out;
use crate::pyerr;
use crate::pyjson::{self, Style};
use crate::pystr;
use crate::scrape::{self, Interrupt, Options, SellerSummary};

fn as_path(text: &str) -> Result<Value, Option<String>> {
    Ok(Value::Str(pyerr::pathlib_normalise(text)))
}

fn positive_int(text: &str) -> Result<Value, Option<String>> {
    let value = pystr::parse_int(text).ok_or(None)?;
    if value < 1 {
        return Err(Some("must be 1 or greater".into()));
    }
    Ok(Value::Int(value))
}

fn positive_float(text: &str) -> Result<Value, Option<String>> {
    let value = pystr::parse_float(text).ok_or(None)?;
    if !value.is_finite() || value <= 0.0 {
        return Err(Some("must be a finite number greater than 0".into()));
    }
    Ok(Value::Float(value))
}

fn int(text: &str) -> Result<Value, Option<String>> {
    pystr::parse_int(text).map(Value::Int).ok_or(None)
}

fn port(text: &str) -> Result<Value, Option<String>> {
    let value = pystr::parse_int(text).ok_or(None)?;
    if !(1..=65535).contains(&value) {
        return Err(Some("port must be between 1 and 65535".into()));
    }
    Ok(Value::Int(value))
}

pub fn build_parser() -> Parser {
    let mut parser = Parser::new("lamp", Some("lamp command line interface."));
    parser.store(
        "--sources",
        as_path,
        "Path",
        Value::Str("sources.toml".into()),
        None,
        None,
    );
    parser.store(
        "--data",
        as_path,
        "Path",
        Value::Str("data".into()),
        None,
        None,
    );

    let mut import = Parser::new("lamp import", None);
    import.positional("dump", as_path, "Path");

    let mut scrape = Parser::new("lamp scrape", None);
    scrape.store(
        "--seller",
        |t| Ok(Value::Str(t.into())),
        "str",
        Value::Str(String::new()),
        None,
        Some("scrape only this seller id"),
    );
    scrape.flag("--full", Some("re-fetch unchanged albums"));
    scrape.store(
        "--refresh-details-days",
        positive_int,
        "_positive_int",
        Value::Int(0),
        Some("N"),
        Some("also refresh unchanged details last checked N days ago"),
    );
    scrape.store(
        "--refresh-limit",
        positive_int,
        "_positive_int",
        Value::Int(100),
        Some("N"),
        Some("maximum extra unchanged-detail requests across this run"),
    );
    scrape.flag(
        "--retry-failed",
        Some("retry only album IDs recorded as failed in earlier runs"),
    );
    scrape.store(
        "--workers",
        positive_int,
        "_positive_int",
        Value::Int(8),
        None,
        None,
    );
    scrape.store(
        "--rps",
        positive_float,
        "_positive_float",
        Value::Float(8.0),
        None,
        Some("global request rate cap"),
    );
    scrape.store(
        "--max-age-days",
        int,
        "int",
        Value::Int(0),
        Some("N"),
        Some("skip albums posted more than N days ago (0 = no age limit); stock that old is usually gone"),
    );
    scrape.flag(
        "--dry-run",
        Some("preview scrape work without fetching album pages or saving files"),
    );

    let mut serve = Parser::new("lamp serve", None);
    serve.store("--port", port, "_port", Value::Int(8000), None, None);
    serve.flag("--no-browser", None);

    let doctor = Parser::new("lamp doctor", None);
    let snapshot = Parser::new("lamp snapshot", None);
    let reindex = Parser::new("lamp reindex", None);
    let mut restore = Parser::new("lamp restore", None);
    restore.positional("archive", as_path, "Path");
    let mut covers = Parser::new("lamp prepare-covers", None);
    covers.store(
        "--limit",
        positive_int,
        "_positive_int",
        Value::Int(1000),
        None,
        None,
    );

    let choice = |name: &str, help: &str, p: Parser| (name.to_string(), Some(help.to_string()), p);
    parser.subparsers(
        "command",
        vec![
            choice("import", "import a raw link dump into sources.toml", import),
            choice("scrape", "scrape Yupoo sellers into data/", scrape),
            choice("serve", "serve the local viewer", serve),
            choice(
                "doctor",
                "check catalogue metadata and cached cover coverage",
                doctor,
            ),
            choice(
                "snapshot",
                "save a bounded metadata snapshot (excludes covers)",
                snapshot,
            ),
            choice(
                "reindex",
                "index the newest duplicate listings from local captures",
                reindex,
            ),
            choice("restore", "restore metadata from a local snapshot", restore),
            choice(
                "prepare-covers",
                "resize and fingerprint cached covers locally",
                covers,
            ),
        ],
    );
    parser
}

struct Args(crate::argparse::Namespace);

impl Args {
    fn str(&self, key: &str) -> String {
        match self.0.get(key) {
            Some(Value::Str(s)) => s.clone(),
            _ => String::new(),
        }
    }
    fn path(&self, key: &str) -> PathBuf {
        PathBuf::from(self.str(key))
    }
    fn int(&self, key: &str) -> i128 {
        match self.0.get(key) {
            Some(Value::Int(i)) => *i,
            _ => 0,
        }
    }
    fn float(&self, key: &str) -> f64 {
        match self.0.get(key) {
            Some(Value::Float(f)) => *f,
            _ => 0.0,
        }
    }
    fn flag(&self, key: &str) -> bool {
        matches!(self.0.get(key), Some(Value::Bool(true)))
    }
}

/// Text encoding of a raw dump, by byte order mark.
fn sniff_encoding(blob: &[u8]) -> &'static str {
    if blob.starts_with(&[0xff, 0xfe]) {
        "utf-16-le"
    } else if blob.starts_with(&[0xfe, 0xff]) {
        "utf-16-be"
    } else if blob.starts_with(&[0xef, 0xbb, 0xbf]) {
        "utf-8-sig"
    } else {
        "utf-8"
    }
}

fn cmd_import(args: &Args) -> i32 {
    let dump = args.path("dump");
    let sources_path = args.path("sources");
    if !dump.exists() {
        out::err(&format!(
            "error: dump file not found: {}",
            pyerr::path_str(&dump)
        ));
        return 1;
    }
    let blob = match pyerr::read_bytes(&dump) {
        Ok(blob) => blob,
        Err(fail) => {
            out::err(&format!("error: {}", fail.message));
            return 1;
        }
    };
    let encoding = sniff_encoding(&blob);
    let decoded = match encoding {
        "utf-16-le" => pyerr::decode_utf16(&blob, false, encoding),
        "utf-16-be" => pyerr::decode_utf16(&blob, true, encoding),
        "utf-8-sig" => pyerr::decode_utf8(&blob[3..]),
        _ => pyerr::decode_utf8(&blob),
    };
    let text = match decoded {
        Ok(text) => text,
        Err(failure) => {
            out::err(&format!(
                "error: could not read {} as {encoding} ({} at byte {}); re-save it as UTF-8 and try again",
                pyerr::path_str(&dump),
                failure.reason,
                failure.start
            ));
            return 1;
        }
    };
    let raws = parse_dump(&text);
    let result = load_sources(&sources_path).and_then(|existing| {
        let (merged, report) = merge_sources(&existing, &raws);
        write_sources(&sources_path, &merged)?;
        Ok(report)
    });
    let report = match result {
        Ok(report) => report,
        Err(e) => {
            out::err(&format!("error: {e}"));
            return 1;
        }
    };
    let mut platforms = String::new();
    if !report.shops_by_platform.is_empty() {
        let mut items = report.shops_by_platform.clone();
        items.sort();
        let detail: Vec<String> = items.iter().map(|(k, v)| format!("{k} {v}")).collect();
        platforms = format!(" ({})", detail.join(", "));
    }
    out::out(&format!(
        "{} rows read, {} invalid\nyupoo sellers: {} new, {} known\nshops: {} new, {} known{platforms}\nwrote {}",
        report.rows,
        report.invalid,
        report.yupoo_new,
        report.yupoo_known,
        report.shops_new,
        report.shops_known,
        pyerr::path_str(&sources_path)
    ));
    if !report.collisions.is_empty() {
        out::err(&format!(
            "{} id collision(s), second URL not stored:",
            report.collisions.len()
        ));
        for (key, kept, dropped) in &report.collisions {
            out::err(&format!("  {key}: kept {kept}, dropped {dropped}"));
        }
    }
    0
}

fn run_timestamp() -> String {
    #[cfg(feature = "fixture-transport")]
    if let Ok(fixed) = std::env::var("LAMP_FIXED_NOW") {
        return fixed;
    }
    chrono::Local::now()
        .format("%Y-%m-%dT%H:%M:%S%:z")
        .to_string()
}

pub fn report_line(s: &SellerSummary, reported: usize, total: usize, dry_run: bool) -> String {
    let verb = if dry_run { "would scrape" } else { "scraped" };
    let mut line = format!(
        "[{reported}/{total}] {}: {} {verb}, {} skipped, {} failed",
        s.seller, s.scraped, s.skipped, s.failed
    );
    if s.skipped_old != 0 {
        line.push_str(&format!(", {} too old", s.skipped_old));
    }
    if s.thumbs_refetched != 0 {
        line.push_str(&format!(", {} covers fetched again", s.thumbs_refetched));
    }
    if !s.failed_ids.is_empty() {
        let shown = s
            .failed_ids
            .iter()
            .take(5)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        let more = if s.failed_ids.len() > 5 { ", ..." } else { "" };
        line.push_str(&format!(" (failed albums: {shown}{more})"));
    }
    if s.thumbs_blocked {
        line.push_str(" (thumbs blocked - hotlink protection?)");
    }
    if s.scraped != 0 && s.dated == 0 && !dry_run {
        line.push_str(" (no publish dates found - Yupoo markup change?)");
    }
    if s.refreshed != 0 || s.deferred != 0 {
        line.push_str(&format!(
            " ({} details refreshed, {} deferred by budget)",
            s.refreshed, s.deferred
        ));
    }
    if !s.error.is_empty() {
        line.push_str(&format!("  ERROR: {}", s.error));
    }
    line
}

fn cmd_scrape(args: &Args) -> i32 {
    let sources = match load_sources(&args.path("sources")) {
        Ok(sources) => sources,
        Err(e) => {
            out::err(&format!("error: {e}"));
            return 1;
        }
    };
    if !sources.values().any(|s| s.kind == "yupoo") {
        out::err("error: no sources - run 'lamp import' first");
        return 1;
    }
    let seller = args.str("seller");
    let only: Option<HashSet<String>> = if seller.is_empty() {
        None
    } else {
        if !sources.contains_key(&format!("yupoo:{seller}")) {
            out::err(&format!("error: unknown seller: {seller}"));
            return 1;
        }
        Some(HashSet::from([seller]))
    };
    let run_ts = run_timestamp();
    let total = match &only {
        Some(only) => only.len(),
        None => sources
            .values()
            .filter(|s| s.kind == "yupoo" && !s.skip)
            .count(),
    };
    let dry_run = args.flag("dry_run");
    let options = Options {
        full: args.flag("full"),
        max_age_days: args.int("max_age_days"),
        dry_run,
        refresh_days: args.int("refresh_details_days"),
        refresh_limit: args.int("refresh_limit"),
        retry_failed: args.flag("retry_failed"),
    };
    let interrupt = Arc::new(Interrupt::default());
    {
        let interrupt = interrupt.clone();
        let _ = ctrlc::set_handler(move || interrupt.press());
    }
    let mut reported = 0usize;
    let mut on_done = |s: &SellerSummary| {
        reported += 1;
        out::out(&report_line(s, reported, total, dry_run));
    };
    let outcome = scrape::scrape_all(
        &sources,
        &args.path("data"),
        &run_ts,
        &options,
        args.int("workers"),
        args.float("rps"),
        only.as_ref(),
        None,
        &mut on_done,
        &interrupt,
    );
    let outcome = match outcome {
        Ok(outcome) => outcome,
        Err(e) => {
            out::err(&format!("error: {e}"));
            return 1;
        }
    };
    if outcome.interrupted {
        let saved = if dry_run {
            ""
        } else {
            "; finished work is saved and index.json is written"
        };
        out::err(&format!(
            "interrupted after {reported} of {total} sellers{saved}"
        ));
        return 130;
    }
    let errored = outcome
        .summaries
        .iter()
        .filter(|s| !s.error.is_empty())
        .count();
    let mut skipped: Vec<&str> = sources
        .values()
        .filter(|s| s.kind == "yupoo" && s.skip && only.as_ref().is_none_or(|o| !o.contains(&s.id)))
        .map(|s| s.id.as_str())
        .collect();
    skipped.sort();
    let tail = if skipped.is_empty() {
        String::new()
    } else {
        format!(", {} skipped ({})", skipped.len(), skipped.join(", "))
    };
    let dry_tag = if dry_run {
        " [DRY RUN - no files written]"
    } else {
        ""
    };
    out::out(&format!(
        "{} sellers, {errored} with errors{tail}{dry_tag}",
        outcome.summaries.len()
    ));
    i32::from(errored > 0)
}

fn maintenance(command: &str, args: &Args) -> Result<i32, Error> {
    let data = args.path("data");
    match command {
        "doctor" => {
            let report = crate::maintenance::diagnose(&data);
            out::out(&pyjson::dumps(&report.clone().into(), Style::Indent(2)));
            let issues = report
                .get("issues")
                .and_then(|v| v.as_array())
                .is_some_and(|i| !i.is_empty());
            return Ok(i32::from(issues));
        }
        "reindex" => {
            let (indexed, duplicates) = crate::storage::reindex(&data)?;
            let map = crate::obj! { "indexed" => pyjson::Value::Int(indexed), "duplicates" => duplicates };
            out::out(&pyjson::dumps(&map.into(), Style::Default));
        }
        "snapshot" => {
            let path = crate::maintenance::snapshot(&data)?;
            out::out(&pyerr::path_str(&path));
        }
        "restore" => {
            let previous = crate::maintenance::restore(&data, &args.path("archive"))?;
            let shown =
                previous.map_or_else(|| "empty target".to_string(), |p| pyerr::path_str(&p));
            out::out(&format!(
                "Metadata restored. Previous files saved at: {shown}"
            ));
        }
        _ => {
            let report = crate::maintenance::prepare_covers(&data, args.int("limit"))?;
            out::out(&pyjson::dumps(&report.clone().into(), Style::Indent(2)));
            let errors = report
                .get("errors")
                .and_then(|v| v.as_array())
                .is_some_and(|e| !e.is_empty());
            return Ok(i32::from(errors));
        }
    }
    Ok(0)
}

fn cmd_serve(args: &Args) -> i32 {
    let data = args.path("data");
    if !data.join("index.json").exists() {
        out::err(&format!(
            "note: no index.json under {} - the viewer will show an empty catalog until 'lamp scrape' writes one",
            pyerr::path_str(&data)
        ));
    }
    let port = u16::try_from(args.int("port")).unwrap_or(8000);
    let server = match crate::serve::make_server(crate::serve::Viewer::Embedded, &data, port) {
        Ok(server) => server,
        Err(e) => {
            out::err(&format!("error: could not listen on 127.0.0.1:{port}: {e}"));
            return 1;
        }
    };
    let stop = Arc::new(AtomicBool::new(false));
    {
        let stop = stop.clone();
        let _ = ctrlc::set_handler(move || stop.store(true, Ordering::SeqCst));
    }
    crate::serve::serve(&server, !args.flag("no_browser"), stop);
    0
}

/// Run the CLI with arguments after the program name; returns the exit code.
pub fn main(argv: &[String]) -> i32 {
    let parser = build_parser();
    let namespace = match parser.parse_args(argv) {
        Ok(namespace) => namespace,
        Err(exit) => {
            out::out_raw(&exit.stdout);
            out::err_raw(&exit.stderr);
            return exit.code;
        }
    };
    let args = Args(namespace);
    let command = args.str("command");
    match command.as_str() {
        "import" => cmd_import(&args),
        "scrape" => cmd_scrape(&args),
        "serve" => cmd_serve(&args),
        other => match maintenance(other, &args) {
            Ok(code) => code,
            Err(e) => {
                out::err(&format!("error: {e}"));
                1
            }
        },
    }
}
