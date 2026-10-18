//! Ported from tests/test_age.py.
mod common;

use lamp::age::{ANCHORS, estimate_published, is_stale, published_date};
use lamp::isodate::Date;

fn date(y: i64, m: i64, d: i64) -> Date {
    Date::from_ymd(y, m, d).unwrap()
}

fn anchor(i: usize) -> (u128, Date) {
    let (id, (y, m, d)) = ANCHORS[i];
    (id, date(y, m, d))
}

#[test]
fn anchors_are_strictly_increasing() {
    for i in 1..ANCHORS.len() {
        assert!(anchor(i).0 > anchor(i - 1).0 && anchor(i).1 > anchor(i - 1).1);
    }
}

#[test]
fn rust_and_viewer_anchor_tables_match() {
    let source = common::read(&common::root().join("viewer").join("age.js"));
    let start = source.find("AGE_ANCHORS = [").unwrap();
    let block = &source[start..start + source[start..].find("];").unwrap()];
    let re = regex::Regex::new(r"\[(\d+),\s*Date\.UTC\((\d+),\s*(\d+),\s*(\d+)\)\]").unwrap();
    let js: Vec<(u128, Date)> = re
        .captures_iter(block)
        .map(|c| {
            (
                c[1].parse().unwrap(),
                date(
                    c[2].parse().unwrap(),
                    c[3].parse::<i64>().unwrap() + 1,
                    c[4].parse().unwrap(),
                ),
            )
        })
        .collect();
    let rust: Vec<(u128, Date)> = (0..ANCHORS.len()).map(anchor).collect();
    assert_eq!(js, rust);
}

#[test]
fn estimate_hits_anchors_interpolates_clamps_and_extrapolates() {
    for i in 0..ANCHORS.len() {
        assert_eq!(estimate_published(anchor(i).0), anchor(i).1);
    }
    let (id0, d0) = anchor(3);
    let (id1, d1) = anchor(4);
    let mid = estimate_published((id0 + id1) / 2);
    assert!(d0 < mid && mid < d1);
    assert_eq!(estimate_published(1), anchor(0).1);
    let (last_id, last_date) = anchor(ANCHORS.len() - 1);
    assert!(estimate_published(last_id + 1_000_000) > last_date);
}

#[test]
fn junk_ids_cap_instead_of_crashing() {
    assert_eq!(estimate_published(10u128.pow(12)), Date::max());
    assert!(!is_stale(10u128.pow(12), 30, date(2026, 8, 18), ""));
}

#[test]
fn published_date_prefers_a_known_date() {
    assert_eq!(published_date(8348553, "2026-01-15"), date(2026, 1, 15));
    assert_eq!(
        published_date(8348553, "2026-01-15T10:00:00+10:00"),
        date(2026, 1, 15)
    );
    assert_eq!(published_date(8348553, "unknown"), anchor(0).1);
    assert_eq!(published_date(8348553, ""), anchor(0).1);
}

#[test]
fn is_stale_cutoffs_known_dates_and_boundary() {
    let today = date(2026, 8, 18);
    let old = anchor(0).0;
    let fresh = anchor(ANCHORS.len() - 1).0;
    assert!(is_stale(old, 180, today, ""));
    assert!(!is_stale(fresh, 180, today, ""));
    assert!(!is_stale(old, 180, today, "2026-08-01"));
    assert!(is_stale(fresh, 180, today, "2017-01-01"));
    assert!(!is_stale(old, 0, today, ""));
    let (id, published) = anchor(ANCHORS.len() - 2);
    let days_old = i128::from(today.ordinal - published.ordinal);
    assert!(!is_stale(id, days_old, today, ""));
    assert!(is_stale(id, days_old - 1, today, ""));
}

#[test]
fn rust_and_viewer_estimates_agree_between_anchors() {
    let mut probes: Vec<u128> = Vec::new();
    for i in 1..ANCHORS.len() {
        let (id0, _) = anchor(i - 1);
        let span = anchor(i).0 - id0;
        probes.extend([id0 + span / 4, id0 + span / 2, id0 + 3 * span / 4]);
    }
    probes.extend([1, anchor(0).0 - 1, anchor(ANCHORS.len() - 1).0 + 5_000_000]);
    let ids = probes
        .iter()
        .map(|p| p.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let script = format!(
        "const {{ estimatePublishedMs }} = require('./viewer/age.js'); const ids = [{ids}]; \
         console.log(ids.map((id) => new Date(estimatePublishedMs(id)).toISOString().slice(0, 10)).join(','));"
    );
    let js = common::run_node(&script, "");
    let rust: Vec<String> = probes
        .iter()
        .map(|&p| estimate_published(p).isoformat())
        .collect();
    assert_eq!(js.trim().split(',').collect::<Vec<_>>(), rust);
}

#[test]
fn rust_and_viewer_agree_on_the_keep_decision() {
    let today = date(2026, 8, 18);
    let mut cases = Vec::new();
    for window in [30i64, 90, 180, 365] {
        for offset in [window - 1, window, window + 1, 0, window + 40] {
            cases.push((
                window,
                Date {
                    ordinal: today.ordinal - offset,
                }
                .isoformat(),
            ));
        }
    }
    let list = cases
        .iter()
        .map(|(w, d)| format!("[{w},\"{d}\"]"))
        .collect::<Vec<_>>()
        .join(",");
    // Local mid-afternoon on the reference day, as production stamps runs.
    let script = format!(
        "const {{ withinAgeWindow }} = require('./viewer/age.js'); const now = new Date(2026, 7, 18, 15, 30).getTime(); \
         console.log([{list}].map(([w, d]) => withinAgeWindow({{ id: '250000000', date_published: d }}, w, now)).join(','));"
    );
    let js = common::run_node(&script, "");
    let rust: Vec<String> = cases
        .iter()
        .map(|(w, d)| (!is_stale(250_000_000, i128::from(*w), today, d)).to_string())
        .collect();
    assert_eq!(js.trim().split(',').collect::<Vec<_>>(), rust);
    assert!(!is_stale(
        250_000_000,
        180,
        today,
        &Date {
            ordinal: today.ordinal - 180
        }
        .isoformat()
    ));
}
