//! Ported from tests/test_scrape.py.
mod common;

use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use common::{json, obj, read_json, write};
use lamp::http::FetchError;
use lamp::ingest::{Source, Sources};
use lamp::pyjson::{Map, Value};
use lamp::scrape::{
    ClientFactory, Interrupt, Options, RefreshBudget, ScrapeClient, SellerSummary, scrape_all,
    scrape_seller,
};
use lamp::storage::{Shard, load_shard, read_run, write_shard};

const EMPTY_PAGE: &str = "<html><body></body></html>";

fn list_item(aid: &str, title: &str) -> String {
    format!(
        "\n<a class=\"album__main\" href=\"/albums/{aid}?uid=1\" title=\"{title}\">\n  <img class=\"album__img\" data-src=\"//photo.yupoo.com/s/{aid}/small.jpg\">\n  <span class=\"album__count\">3</span>\n  <div class=\"album__title\">{title}</div>\n</a>\n"
    )
}

fn no_cover_item(aid: &str, title: &str) -> String {
    format!(
        "<a class=\"album__main\" href=\"/albums/{aid}?uid=1\" title=\"{title}\"><img class=\"album__img\" data-src=\"https://cdn.example.com/yupoo/album/im_photo_album.png\"><span class=\"album__count\">3</span><div class=\"album__title\">{title}</div></a>"
    )
}

fn album_page(title: &str, desc: &str) -> String {
    format!(
        "\n<span class=\"showalbumheader__gallerytitle\" data-name=\"{title}\">{title}</span>\n<div class=\"showalbumheader__gallerysubtitle\">{desc}</div>\n<span class=\"showalbumheader__gallerycount\">3</span>\n"
    )
}

type Hook = Arc<dyn Fn(&str) + Send + Sync>;

#[derive(Default)]
struct Inner {
    pages: Mutex<Vec<(String, String)>>,
    blocked: Mutex<Vec<String>>,
    fetch_errors: Mutex<Vec<String>>,
    calls: Mutex<Vec<String>>,
    downloads: Mutex<Vec<String>>,
    block_pattern: Mutex<Vec<bool>>,
    panic_on: Mutex<Option<String>>,
    hang_on: Mutex<Option<(String, Arc<AtomicBool>)>>,
    hook: Mutex<Option<Hook>>,
}

/// URL substrings mapped to pages; downloads succeed unless the block
/// pattern (indexed by download order) says otherwise.
#[derive(Clone, Default)]
struct FakeClient(Arc<Inner>);

impl FakeClient {
    fn new(pages: &[(&str, String)]) -> FakeClient {
        let client = FakeClient::default();
        for (marker, text) in pages {
            client.set(marker, text.clone());
        }
        client
    }
    fn set(&self, marker: &str, text: String) {
        let mut pages = self.0.pages.lock().unwrap();
        match pages.iter_mut().find(|(m, _)| m == marker) {
            Some(entry) => entry.1 = text,
            None => pages.push((marker.to_string(), text)),
        }
    }
    fn blocked(self, markers: &[&str]) -> Self {
        *self.0.blocked.lock().unwrap() = markers.iter().map(|s| s.to_string()).collect();
        self
    }
    fn fetch_errors(self, markers: &[&str]) -> Self {
        *self.0.fetch_errors.lock().unwrap() = markers.iter().map(|s| s.to_string()).collect();
        self
    }
    fn block_downloads(self, pattern: &[bool]) -> Self {
        *self.0.block_pattern.lock().unwrap() = pattern.to_vec();
        self
    }
    fn calls(&self) -> Vec<String> {
        self.0.calls.lock().unwrap().clone()
    }
    fn downloads(&self) -> Vec<String> {
        self.0.downloads.lock().unwrap().clone()
    }
    fn called(&self, needle: &str) -> bool {
        self.calls().iter().any(|u| u.contains(needle))
    }
    fn factory(&self) -> ClientFactory {
        let client = self.clone();
        Arc::new(move || Box::new(client.clone()) as Box<dyn ScrapeClient>)
    }
}

impl ScrapeClient for FakeClient {
    fn get(&self, url: &str, _referer: Option<&str>) -> Result<String, FetchError> {
        self.0.calls.lock().unwrap().push(url.to_string());
        if let Some(hook) = self.0.hook.lock().unwrap().clone() {
            hook(url);
        }
        let panic_on = self.0.panic_on.lock().unwrap().clone();
        if panic_on.is_some_and(|marker| url.contains(&marker)) {
            panic!("parser bug");
        }
        let hang_on = self.0.hang_on.lock().unwrap().clone();
        if let Some((marker, release)) = hang_on
            && url.contains(&marker)
        {
            for _ in 0..1000 {
                if release.load(std::sync::atomic::Ordering::SeqCst) {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
        if self
            .0
            .blocked
            .lock()
            .unwrap()
            .iter()
            .any(|m| url.contains(m.as_str()))
        {
            return Err(FetchError::Blocked(429));
        }
        if self
            .0
            .fetch_errors
            .lock()
            .unwrap()
            .iter()
            .any(|m| url.contains(m.as_str()))
        {
            return Err(FetchError::Fetch(format!("fake fetch error for {url}")));
        }
        let pages = self.0.pages.lock().unwrap();
        Ok(pages
            .iter()
            .find(|(m, _)| url.contains(m.as_str()))
            .map_or(EMPTY_PAGE.to_string(), |(_, t)| t.clone()))
    }

    fn download(&self, url: &str, dest: &Path, _referer: Option<&str>) -> Result<bool, FetchError> {
        let index = {
            let mut downloads = self.0.downloads.lock().unwrap();
            downloads.push(url.to_string());
            downloads.len() - 1
        };
        if self
            .0
            .block_pattern
            .lock()
            .unwrap()
            .get(index)
            .copied()
            .unwrap_or(false)
        {
            return Err(FetchError::Blocked(403));
        }
        write(dest, b"jpg");
        Ok(true)
    }
}

fn source(id: &str, skip: bool) -> Source {
    let mut s = Source::new(
        "yupoo",
        id,
        "yupoo",
        &format!("https://{id}.x.yupoo.com"),
        id,
    );
    s.skip = skip;
    s
}

fn seller() -> Source {
    source("sellerx", false)
}

fn sources(list: &[Source]) -> Sources {
    list.iter()
        .map(|s| (format!("{}:{}", s.kind, s.id), s.clone()))
        .collect()
}

fn make_client() -> FakeClient {
    let list_html = list_item("101", "Item A")
        + &list_item("102", "Item B https://item.taobao.com/item.htm?id=7");
    let mut pages: Vec<(String, String)> = Vec::new();
    for (name, first, second) in [("sellery", "201", "202"), ("skipme", "301", "302")] {
        pages.push((
            format!("{name}.x.yupoo.com/albums?page=1"),
            list_html.replace("101", first).replace("102", second),
        ));
    }
    for (aid, title, desc) in [
        ("201", "Item A", "weidian.com/item.html?itemID=5"),
        ("202", "Item B", "plain text"),
        ("301", "Item A", "weidian.com/item.html?itemID=5"),
        ("302", "Item B", "plain text"),
    ] {
        pages.push((format!("/albums/{aid}"), album_page(title, desc)));
    }
    pages.push(("albums?page=1".into(), list_html));
    pages.push(("albums?page=2".into(), EMPTY_PAGE.into()));
    pages.push((
        "/albums/101".into(),
        album_page("Item A", "weidian.com/item.html?itemID=5"),
    ));
    pages.push(("/albums/102".into(), album_page("Item B", "plain text")));
    let refs: Vec<(&str, String)> = pages.iter().map(|(m, t)| (m.as_str(), t.clone())).collect();
    FakeClient::new(&refs)
}

fn run(client: &FakeClient, dir: &Path, run_ts: &str, options: Options) -> SellerSummary {
    run_source(client, &seller(), dir, run_ts, options)
}

fn run_source(
    client: &FakeClient,
    source: &Source,
    dir: &Path,
    run_ts: &str,
    options: Options,
) -> SellerSummary {
    let budget = RefreshBudget::new(options.refresh_limit);
    scrape_seller(
        client,
        source,
        dir,
        run_ts,
        &options,
        &AtomicBool::new(false),
        &budget,
    )
    .unwrap()
}

fn opts() -> Options {
    Options::default()
}

fn all(
    list: &[Source],
    dir: &Path,
    run_ts: &str,
    client: &FakeClient,
    options: Options,
    workers: i128,
    only: Option<&[&str]>,
) -> Vec<SellerSummary> {
    let only: Option<HashSet<String>> = only.map(|o| o.iter().map(|s| s.to_string()).collect());
    let interrupt = Interrupt::default();
    scrape_all(
        &sources(list),
        dir,
        run_ts,
        &options,
        workers,
        8.0,
        only.as_ref(),
        Some(client.factory()),
        &mut |_| {},
        &interrupt,
    )
    .unwrap()
    .summaries
}

fn shard(dir: &Path) -> Shard {
    load_shard(dir, "sellerx").unwrap()
}

fn get<'a>(shard: &'a Shard, id: &str, key: &str) -> &'a Value {
    &shard[id][key]
}

fn index(dir: &Path) -> Map {
    obj(&read_json(&dir.join("index.json"))).clone()
}

fn index_sellers(dir: &Path) -> Vec<(String, String)> {
    index(dir)["sellers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| (obj(s)["id"].py_str(), obj(s)["album_count"].py_str()))
        .collect()
}

const T1: &str = "2026-08-17T12:00:00+10:00";
const T2: &str = "2026-08-18T12:00:00+10:00";

#[test]
fn unchanged_details_refresh_with_a_shared_budget() {
    let dir = common::tmp();
    run(
        &make_client(),
        dir.path(),
        "2026-08-01T00:00:00+00:00",
        opts(),
    );
    let mut saved = shard(dir.path());
    for album in saved.values_mut() {
        album.insert("price".into(), Value::Int(180));
    }
    write_shard(dir.path(), "sellerx", &saved).unwrap();
    let client = make_client();
    client.set("/albums/101", album_page("Item A", "CNY price: ¥280"));
    let result = run(
        &client,
        dir.path(),
        "2026-09-26T00:00:00+00:00",
        Options {
            refresh_days: 14,
            refresh_limit: 1,
            ..opts()
        },
    );
    assert_eq!((result.refreshed, result.deferred), (1, 1));
    let saved = shard(dir.path());
    assert!(matches!(get(&saved, "101", "price"), Value::Int(280)));
    assert_eq!(
        obj(get(&saved, "101", "price_evidence"))["token"].py_str(),
        "¥280"
    );
    assert!(matches!(get(&saved, "102", "price"), Value::Int(180)));
    assert_eq!(
        get(&saved, "102", "updated_at").py_str(),
        "2026-08-01T00:00:00+00:00"
    );
    assert_eq!(
        get(&saved, "102", "last_seen").py_str(),
        "2026-09-26T00:00:00+00:00"
    );
}

#[test]
fn refresh_dry_run_counts_without_writing() {
    let dir = common::tmp();
    run(
        &make_client(),
        dir.path(),
        "2026-08-01T00:00:00+00:00",
        opts(),
    );
    let before = std::fs::read(dir.path().join("catalog/sellerx.json")).unwrap();
    let client = make_client();
    let result = run(
        &client,
        dir.path(),
        "2026-09-26T00:00:00+00:00",
        Options {
            refresh_days: 14,
            refresh_limit: 1,
            dry_run: true,
            ..opts()
        },
    );
    assert_eq!((result.refreshed, result.deferred), (1, 1));
    assert!(client.calls().iter().all(|u| u.contains("albums?page=")));
    assert_eq!(
        std::fs::read(dir.path().join("catalog/sellerx.json")).unwrap(),
        before
    );
}

#[test]
fn failed_retry_is_persisted_and_targeted() {
    let dir = common::tmp();
    let broken = make_client().fetch_errors(&["/albums/101"]);
    all(
        &[seller()],
        dir.path(),
        "2026-09-01T00:00:00+00:00",
        &broken,
        opts(),
        4,
        None,
    );
    let first = read_run(dir.path(), "sellerx").unwrap();
    assert_eq!(first["failed_ids"].py_str(), r#"["101"]"#);
    assert!(!first["last_success"].truthy());
    let client = make_client();
    all(
        &[seller()],
        dir.path(),
        "2026-09-26T00:00:00+00:00",
        &client,
        Options {
            retry_failed: true,
            ..opts()
        },
        4,
        None,
    );
    let latest = read_run(dir.path(), "sellerx").unwrap();
    assert_eq!(latest["failed_ids"].py_str(), "[]");
    assert_eq!(latest["last_success"].py_str(), "2026-09-26T00:00:00+00:00");
    assert!(!client.called("/albums/102?"));
}

#[test]
fn delisted_failure_clears_only_after_a_complete_listing() {
    for complete in [true, false] {
        let dir = common::tmp();
        let broken = make_client().fetch_errors(&["/albums/101"]);
        all(
            &[seller()],
            dir.path(),
            "2026-09-01T00:00:00+00:00",
            &broken,
            opts(),
            4,
            None,
        );
        assert_eq!(
            read_run(dir.path(), "sellerx").unwrap()["failed_ids"].py_str(),
            r#"["101"]"#
        );
        let mut client = make_client();
        client.set("albums?page=1", list_item("102", "Item B"));
        if !complete {
            client = client.fetch_errors(&["albums?page=2"]);
        }
        all(
            &[seller()],
            dir.path(),
            "2026-09-26T00:00:00+00:00",
            &client,
            Options {
                retry_failed: true,
                ..opts()
            },
            4,
            None,
        );
        let latest = read_run(dir.path(), "sellerx").unwrap();
        assert_eq!(
            latest["failed_ids"].py_str(),
            if complete { "[]" } else { r#"["101"]"# }
        );
        assert_eq!(
            latest["last_success"].py_str(),
            if complete {
                "2026-09-26T00:00:00+00:00"
            } else {
                ""
            }
        );
        assert_eq!(
            latest["last_listing"].py_str(),
            if complete {
                "2026-09-26T00:00:00+00:00"
            } else {
                "2026-09-01T00:00:00+00:00"
            }
        );
        assert!(!client.called("/albums/101?"));
    }
}

#[test]
fn empty_detail_response_keeps_last_usable_record() {
    let dir = common::tmp();
    run(
        &make_client(),
        dir.path(),
        "2026-08-01T00:00:00+00:00",
        opts(),
    );
    let before = get(&shard(dir.path()), "101", "updated_at").py_str();
    let client = make_client();
    client.set("/albums/101", EMPTY_PAGE.into());
    let summary = run(
        &client,
        dir.path(),
        "2026-09-26T00:00:00+00:00",
        Options {
            full: true,
            ..opts()
        },
    );
    assert!(summary.failed_ids.contains(&"101".to_string()));
    assert_eq!(
        get(&shard(dir.path()), "101", "updated_at").py_str(),
        before
    );
}

#[test]
fn full_pass_records_titles_links_and_covers() {
    let dir = common::tmp();
    let summary = run(&make_client(), dir.path(), T1, opts());
    assert_eq!(
        (
            summary.scraped,
            summary.skipped,
            summary.failed,
            summary.error.as_str()
        ),
        (2, 0, 0, "")
    );
    let s = shard(dir.path());
    assert_eq!(
        s.keys().cloned().collect::<HashSet<_>>(),
        ["101", "102"].iter().map(|x| x.to_string()).collect()
    );
    assert_eq!(get(&s, "101", "title").py_str(), "Item A");
    assert_eq!(get(&s, "101", "list_title").py_str(), "Item A");
    assert_eq!(get(&s, "102", "title").py_str(), "Item B");
    assert_eq!(
        get(&s, "102", "list_title").py_str(),
        "Item B https://item.taobao.com/item.htm?id=7"
    );
    assert_eq!(
        get(&s, "101", "links").py_str(),
        r#"[{"platform": "weidian", "url": "https://weidian.com/item.html?itemID=5"}]"#
    );
    assert_eq!(
        get(&s, "102", "links").py_str(),
        r#"[{"platform": "taobao", "url": "https://item.taobao.com/item.htm?id=7"}]"#
    );
    assert_eq!(get(&s, "101", "thumb").py_str(), "thumbs/sellerx/101.jpg");
    assert_eq!(
        get(&s, "101", "album_url").py_str(),
        "https://sellerx.x.yupoo.com/albums/101?uid=1"
    );
    assert_eq!(get(&s, "101", "first_seen").py_str(), T1);
    assert!(dir.path().join("thumbs/sellerx/101.jpg").exists());
}

/// A listing whose pages carry the given (first, last) album ID ranges, with a
/// detail page for each.
fn paged(pages: &[(u32, u32)]) -> FakeClient {
    let client = FakeClient::default();
    for (number, &(first, last)) in pages.iter().enumerate() {
        let items = (first..=last)
            .map(|id| list_item(&id.to_string(), &format!("Item {id}")))
            .collect();
        client.set(&format!("albums?page={}", number + 1), items);
        for id in first..=last {
            client.set(
                &format!("/albums/{id}"),
                album_page(&format!("Item {id}"), ""),
            );
        }
    }
    client
}

fn run_once(client: &FakeClient, dir: &Path, run_ts: &str, options: Options) -> SellerSummary {
    all(&[seller()], dir, run_ts, client, options, 4, None).remove(0)
}

#[test]
fn short_listing_marks_nothing_gone_until_a_second_run_confirms_it() {
    let dir = common::tmp();
    let mut client = paged(&[(101, 106), (107, 112)]);
    client = client.fetch_errors(&["/albums/112?"]);
    let first = "2026-09-01T00:00:00+00:00";
    all(&[seller()], dir.path(), first, &client, opts(), 4, None);
    assert_eq!(
        read_run(dir.path(), "sellerx").unwrap()["failed_ids"].py_str(),
        r#"["112"]"#
    );

    // Page 2 now answers with no albums, as a soft block would.
    let short = paged(&[(101, 106)]);
    let second = "2026-09-02T00:00:00+00:00";
    let summary = &all(&[seller()], dir.path(), second, &short, opts(), 4, None)[0];
    assert!(!summary.listing_complete);
    assert!(
        summary.error.contains("not marked gone"),
        "{}",
        summary.error
    );
    assert!(
        shard(dir.path())
            .values()
            .all(|a| !a.contains_key("gone_since"))
    );
    let run = read_run(dir.path(), "sellerx").unwrap();
    assert_eq!(run["failed_ids"].py_str(), r#"["112"]"#);
    assert_eq!(run["last_listing"].py_str(), first);
    assert_eq!(get(&shard(dir.path()), "101", "last_seen").py_str(), second);

    let third = "2026-09-03T00:00:00+00:00";
    let summary = &all(&[seller()], dir.path(), third, &short, opts(), 4, None)[0];
    assert!(summary.listing_complete);
    assert_eq!(summary.error, "");
    let s = shard(dir.path());
    for id in 107..=111 {
        assert_eq!(get(&s, &id.to_string(), "gone_since").py_str(), third);
    }
    assert!(!s["101"].contains_key("gone_since"));
    let run = read_run(dir.path(), "sellerx").unwrap();
    assert_eq!(run["failed_ids"].py_str(), "[]");
    assert_eq!(run["last_listing"].py_str(), third);
}

#[test]
fn failed_albums_without_records_count_towards_a_short_listing() {
    let dir = common::tmp();
    let client = paged(&[(101, 106), (107, 112)]).fetch_errors(&[
        "/albums/11",
        "/albums/107",
        "/albums/108",
        "/albums/109",
    ]);
    let first = "2026-09-01T00:00:00+00:00";
    run_once(&client, dir.path(), first, opts());
    let failed = r#"["107", "108", "109", "110", "111", "112"]"#;
    assert_eq!(
        read_run(dir.path(), "sellerx").unwrap()["failed_ids"].py_str(),
        failed
    );
    assert_eq!(shard(dir.path()).len(), 6);

    let summary = run_once(
        &paged(&[(101, 106)]),
        dir.path(),
        "2026-09-02T00:00:00+00:00",
        opts(),
    );
    assert!(!summary.listing_complete);
    let run = read_run(dir.path(), "sellerx").unwrap();
    assert_eq!(run["failed_ids"].py_str(), failed);
    assert_eq!(run["last_listing"].py_str(), first);
}

/// IDs from `first` to `last` as strings.
fn ids(first: u32, last: u32) -> Vec<String> {
    (first..=last).map(|id| id.to_string()).collect()
}

fn gone(dir: &Path) -> Vec<String> {
    let mut ids: Vec<String> = shard(dir)
        .iter()
        .filter(|(_, a)| a.contains_key("gone_since"))
        .map(|(id, _)| id.clone())
        .collect();
    ids.sort();
    ids
}

#[test]
fn only_albums_missing_from_both_short_listings_are_marked_gone() {
    let dir = common::tmp();
    let all_three = paged(&[(101, 106), (107, 112), (113, 118)]);
    run_once(&all_three, dir.path(), T1, opts());
    let summary = run_once(&paged(&[(101, 106), (107, 112)]), dir.path(), T2, opts());
    assert_eq!(summary.short_missing, ids(113, 118));

    // A shorter listing confirms only what the last one also missed.
    let third = "2026-08-19T12:00:00+10:00";
    let summary = run_once(&paged(&[(101, 106)]), dir.path(), third, opts());
    assert!(!summary.listing_complete);
    assert_eq!(summary.short_missing, ids(107, 112));
    assert_eq!(gone(dir.path()), ids(113, 118));

    // A listing as long but shifted confirms none of the albums it misses.
    let dir = common::tmp();
    run_once(&all_three, dir.path(), T1, opts());
    run_once(&paged(&[(101, 106), (107, 112)]), dir.path(), T2, opts());
    let shifted = paged(&[(107, 112), (113, 118)]);
    let summary = run_once(&shifted, dir.path(), third, opts());
    assert_eq!(summary.short_missing, ids(101, 106));
    assert!(gone(dir.path()).is_empty());
}

#[test]
fn a_blocked_run_does_not_confirm_a_short_listing() {
    let dir = common::tmp();
    run_once(&paged(&[(101, 106), (107, 112)]), dir.path(), T1, opts());
    run_once(&paged(&[(101, 106)]), dir.path(), T2, opts());
    let blocked = paged(&[(101, 106)]).blocked(&["/albums/10"]);
    let summary = run_once(
        &blocked,
        dir.path(),
        "2026-08-19T12:00:00+10:00",
        Options {
            full: true,
            ..opts()
        },
    );
    assert!(
        summary.error.starts_with("blocked: aborted"),
        "{}",
        summary.error
    );
    assert!(summary.error.contains("not marked gone"));
    assert!(!summary.listing_complete);
    assert_eq!(summary.short_missing, ids(107, 112));
    assert!(
        shard(dir.path())
            .values()
            .all(|a| !a.contains_key("gone_since"))
    );
}

#[test]
fn incremental_skips_unchanged_and_retains_delisted() {
    let dir = common::tmp();
    run(&make_client(), dir.path(), T1, opts());
    let client = make_client();
    let summary = run(&client, dir.path(), T2, opts());
    assert_eq!((summary.skipped, summary.scraped), (2, 0));
    assert!(!client.called("/albums/101"));
    assert_eq!(get(&shard(dir.path()), "101", "first_seen").py_str(), T1);
    let only_102 = FakeClient::new(&[
        (
            "albums?page=1",
            list_item("102", "Item B https://item.taobao.com/item.htm?id=7"),
        ),
        ("albums?page=2", EMPTY_PAGE.into()),
    ]);
    run(&only_102, dir.path(), T2, opts());
    assert_eq!(shard(dir.path()).len(), 2);
}

#[test]
fn layout_change_guard_preserves_the_shard() {
    let dir = common::tmp();
    run(&make_client(), dir.path(), T1, opts());
    let summary = run(
        &FakeClient::new(&[("albums?page=1", EMPTY_PAGE.into())]),
        dir.path(),
        T2,
        opts(),
    );
    assert!(!summary.error.is_empty());
    assert!(dir.path().join("debug/sellerx-page1.html").exists());
    assert_eq!(shard(dir.path()).len(), 2);
}

#[test]
fn pagination_stops_on_a_repeated_page() {
    let dir = common::tmp();
    let list = list_item("101", "Item A")
        + &list_item("102", "Item B https://item.taobao.com/item.htm?id=7");
    let client = FakeClient::new(&[
        ("albums?page=1", list.clone()),
        ("albums?page=2", list),
        ("albums?page=3", EMPTY_PAGE.into()),
        (
            "/albums/101",
            album_page("Item A", "weidian.com/item.html?itemID=5"),
        ),
        ("/albums/102", album_page("Item B", "plain text")),
    ]);
    let summary = run(&client, dir.path(), T1, opts());
    assert_eq!((summary.error.as_str(), summary.scraped), ("", 2));
    assert!(!client.called("page=3"));
}

#[test]
fn blocked_streaks_abort_the_seller() {
    let dir = common::tmp();
    let summary = run(
        &FakeClient::default().blocked(&["sellerx"]),
        dir.path(),
        T1,
        opts(),
    );
    assert!(summary.error.contains("blocked"));
    let dir = common::tmp();
    let kept = json(
        r#"{"999": {"id": "999", "seller": "sellerx", "title": "kept", "list_title": "kept", "thumb": "thumbs/sellerx/999.jpg", "album_url": "https://sellerx.x.yupoo.com/albums/999", "links": [], "category": "", "photo_count": 1, "first_seen": "2026-08-01T00:00:00+10:00", "updated_at": "2026-08-01T00:00:00+10:00"}}"#,
    );
    let kept: Shard = obj(&kept)
        .iter()
        .map(|(k, v)| (k.clone(), obj(v).clone()))
        .collect();
    write_shard(dir.path(), "sellerx", &kept).unwrap();
    let list: String = ["101", "102", "103", "104"]
        .iter()
        .map(|a| list_item(a, &format!("Item {a}")))
        .collect();
    let client = FakeClient::new(&[
        ("albums?page=1", list),
        ("albums?page=2", EMPTY_PAGE.into()),
    ])
    .blocked(&["/albums/"]);
    let summary = run(&client, dir.path(), T1, opts());
    assert!(summary.error.contains("blocked"));
    assert_eq!(summary.failed, 3);
    let s = shard(dir.path());
    assert_eq!(s.keys().cloned().collect::<Vec<_>>(), ["999"]);
    assert_eq!(get(&s, "999", "title").py_str(), "kept");
}

fn seller_of(aids: &[&str]) -> Vec<(String, String)> {
    let list: String = aids
        .iter()
        .map(|a| list_item(a, &format!("Item {a}")))
        .collect();
    let mut pages = vec![
        ("albums?page=1".to_string(), list),
        ("albums?page=2".to_string(), EMPTY_PAGE.to_string()),
    ];
    for a in aids {
        pages.push((
            format!("/albums/{a}"),
            album_page(&format!("Item {a}"), "plain text"),
        ));
    }
    pages
}

fn client_of(pages: &[(String, String)]) -> FakeClient {
    let refs: Vec<(&str, String)> = pages.iter().map(|(m, t)| (m.as_str(), t.clone())).collect();
    FakeClient::new(&refs)
}

#[test]
fn all_failed_fetches_report_an_error_but_partial_failures_do_not() {
    let dir = common::tmp();
    let list: String = ["101", "102", "103"]
        .iter()
        .map(|a| list_item(a, &format!("Item {a}")))
        .collect();
    let client = FakeClient::new(&[
        ("albums?page=1", list),
        ("albums?page=2", EMPTY_PAGE.into()),
    ])
    .fetch_errors(&["/albums/"]);
    let summary = run(&client, dir.path(), T1, opts());
    assert_eq!(
        (summary.scraped, summary.failed, summary.error.as_str()),
        (0, 3, "all 3 album fetches failed")
    );
    let dir = common::tmp();
    let mut pages = seller_of(&["101", "102", "103"]);
    pages.retain(|(m, _)| m != "/albums/103");
    let summary = run(
        &client_of(&pages).fetch_errors(&["/albums/103"]),
        dir.path(),
        T1,
        opts(),
    );
    assert_eq!(
        (summary.scraped, summary.failed, summary.error.as_str()),
        (2, 1, "")
    );
}

#[test]
fn thumb_blocks_disable_covers_after_three_in_a_row_and_reset_on_success() {
    let dir = common::tmp();
    let client =
        client_of(&seller_of(&["101", "102", "103", "104", "105"])).block_downloads(&[true; 5]);
    let summary = run(&client, dir.path(), T1, opts());
    assert_eq!(
        (
            summary.error.as_str(),
            summary.scraped,
            summary.failed,
            summary.thumbs_blocked
        ),
        ("", 5, 0, true)
    );
    assert_eq!(client.downloads().len(), 3);
    let s = shard(dir.path());
    assert_eq!(s.len(), 5);
    assert_eq!(get(&s, "105", "thumb").py_str(), "thumbs/sellerx/105.jpg");
    let dir = common::tmp();
    let client = client_of(&seller_of(&["101", "102", "103", "104", "105"]))
        .block_downloads(&[true, true, false, true, true]);
    let summary = run(&client, dir.path(), T1, opts());
    assert_eq!(
        (
            summary.error.as_str(),
            summary.scraped,
            summary.thumbs_blocked
        ),
        ("", 5, false)
    );
    assert_eq!(client.downloads().len(), 5);
}

#[test]
fn an_album_with_no_cover_records_no_thumb_path() {
    let dir = common::tmp();
    let client = FakeClient::new(&[
        ("albums?page=1", no_cover_item("501", "No cover")),
        ("albums?page=2", EMPTY_PAGE.into()),
        ("/albums/501", album_page("No cover", "plain text")),
    ]);
    let summary = run(&client, dir.path(), T1, opts());
    assert_eq!(summary.scraped, 1);
    assert!(client.downloads().is_empty());
    assert_eq!(get(&shard(dir.path()), "501", "thumb").py_str(), "");
    assert!(!dir.path().join("thumbs").exists());
}

#[test]
fn scrape_all_writes_the_index_and_keeps_scoped_sellers() {
    let dir = common::tmp();
    let mut list = sources(&[seller()]);
    list.insert(
        "shop:weidian-7".into(),
        Source::new(
            "shop",
            "weidian-7",
            "weidian",
            "https://weidian.com/?userid=7",
            "weidian 7",
        ),
    );
    let interrupt = Interrupt::default();
    let summaries = scrape_all(
        &list,
        dir.path(),
        T1,
        &opts(),
        4,
        8.0,
        None,
        Some(make_client().factory()),
        &mut |_| {},
        &interrupt,
    )
    .unwrap()
    .summaries;
    assert_eq!(
        summaries
            .iter()
            .map(|s| s.seller.as_str())
            .collect::<Vec<_>>(),
        ["sellerx"]
    );
    let i = index(dir.path());
    assert_eq!(i["generated_at"].py_str(), T1);
    assert_eq!(
        index_sellers(dir.path()),
        [("sellerx".to_string(), "2".to_string())]
    );
    assert_eq!(
        obj(&i["sellers"].as_array().unwrap()[0])["shard"].py_str(),
        "catalog/sellerx.json"
    );
    assert_eq!(
        obj(&i["shops"].as_array().unwrap()[0])["id"].py_str(),
        "weidian-7"
    );

    let dir = common::tmp();
    let other = json(
        r#"{"7": {"id": "7", "seller": "othersella", "title": "kept", "list_title": "kept", "thumb": "thumbs/othersella/7.jpg", "album_url": "https://othersella.x.yupoo.com/albums/7", "links": [], "category": "", "photo_count": 1, "first_seen": "2026-08-01T00:00:00+10:00", "updated_at": "2026-08-01T00:00:00+10:00"}}"#,
    );
    write_shard(
        dir.path(),
        "othersella",
        &obj(&other)
            .iter()
            .map(|(k, v)| (k.clone(), obj(v).clone()))
            .collect(),
    )
    .unwrap();
    let summaries = all(
        &[seller(), source("othersella", false)],
        dir.path(),
        T1,
        &make_client(),
        opts(),
        4,
        Some(&["sellerx"]),
    );
    assert_eq!(
        summaries
            .iter()
            .map(|s| s.seller.as_str())
            .collect::<Vec<_>>(),
        ["sellerx"]
    );
    assert_eq!(
        index_sellers(dir.path())[0],
        ("othersella".to_string(), "1".to_string())
    );
    assert_eq!(index_sellers(dir.path())[1].0, "sellerx");
}

fn aged_client() -> FakeClient {
    FakeClient::new(&[
        (
            "albums?page=1",
            list_item("250700000", "Fresh") + &list_item("9000000", "Ancient"),
        ),
        ("albums?page=2", EMPTY_PAGE.into()),
        ("/albums/250700000", album_page("Fresh", "")),
        ("/albums/9000000", album_page("Ancient", "")),
    ])
}

#[test]
fn max_age_days_skips_trusts_stored_dates_and_keeps_old_records() {
    let dir = common::tmp();
    let client = aged_client();
    let summary = run(
        &client,
        dir.path(),
        T2,
        Options {
            max_age_days: 180,
            ..opts()
        },
    );
    assert_eq!((summary.scraped, summary.skipped_old), (1, 1));
    assert!(!client.called("/albums/9000000"));
    assert!(!client.downloads().iter().any(|u| u.contains("9000000")));
    assert_eq!(
        shard(dir.path()).keys().cloned().collect::<Vec<_>>(),
        ["250700000"]
    );

    let dir = common::tmp();
    let summary = run(&aged_client(), dir.path(), T2, opts());
    assert_eq!((summary.scraped, summary.skipped_old), (2, 0));
    run(
        &aged_client(),
        dir.path(),
        T2,
        Options {
            max_age_days: 180,
            full: true,
            ..opts()
        },
    );
    assert_eq!(shard(dir.path()).len(), 2);

    let path = dir.path().join("catalog/sellerx.json");
    let mut payload = read_json(&path);
    for album in payload
        .as_object_mut()
        .unwrap()
        .get_mut("albums")
        .unwrap()
        .as_array_mut()
        .unwrap()
    {
        let album = album.as_object_mut().unwrap();
        if album["id"].py_str() == "9000000" {
            album.insert("date_published".into(), "2026-08-01".into());
        }
    }
    write(
        &path,
        lamp::pyjson::dumps(&payload, lamp::pyjson::Style::Default),
    );
    let client = aged_client();
    let summary = run(
        &client,
        dir.path(),
        T2,
        Options {
            max_age_days: 180,
            full: true,
            ..opts()
        },
    );
    assert_eq!(summary.skipped_old, 0);
    assert!(client.called("/albums/9000000"));
}

fn unreadable_cards(ids: &[&str]) -> String {
    ids.iter()
        .map(|a| format!("<a class=\"mystery__main\" href=\"/albums/{a}\">x</a>"))
        .collect()
}

#[test]
fn unreadable_pages_report_a_layout_change_and_mark_nothing_gone() {
    let dir = common::tmp();
    let summary = run(
        &FakeClient::new(&[(
            "albums?page=1",
            format!(
                "<html><body>{}</body></html>",
                unreadable_cards(&["901", "902", "903"])
            ),
        )]),
        dir.path(),
        T2,
        opts(),
    );
    assert!(summary.error.contains("layout change"));
    assert!(dir.path().join("debug/sellerx-page1.html").exists());
    for page2 in [
        unreadable_cards(&["102", "103", "104"]),
        list_item("105", "Item E") + &unreadable_cards(&["102", "103", "104"]),
    ] {
        let dir = common::tmp();
        run(&make_client(), dir.path(), T1, opts());
        let client = FakeClient::new(&[
            ("albums?page=1", list_item("101", "Item A")),
            ("albums?page=2", page2),
        ]);
        let summary = run(&client, dir.path(), T2, opts());
        assert!(summary.error.contains("layout change"));
        assert!(!summary.listing_complete);
        assert!(dir.path().join("debug/sellerx-page2.html").exists());
        let s = shard(dir.path());
        assert_eq!(s.len(), 2);
        assert!(s.values().all(|a| !a.contains_key("gone_since")));
    }
}

#[test]
fn empty_pages_report_an_empty_seller_not_a_layout_change() {
    for html in [
        "<html><body><a href=\"/albums/12345\">featured</a></body></html>",
        EMPTY_PAGE,
    ] {
        let dir = common::tmp();
        let summary = run(
            &FakeClient::new(&[("albums?page=1", html.into())]),
            dir.path(),
            T2,
            opts(),
        );
        assert!(summary.error.contains("empty or closed seller"));
        assert!(!summary.error.contains("layout change"));
    }
    let dir = common::tmp();
    let summary = run(
        &FakeClient::new(&[("albums?page=1", EMPTY_PAGE.into())]),
        dir.path(),
        T2,
        Options {
            dry_run: true,
            ..opts()
        },
    );
    assert!(summary.error.contains("empty or closed seller"));
    assert!(!dir.path().join("debug").exists());
}

#[test]
fn skip_is_honoured_except_for_an_explicit_seller_and_the_index_follows() {
    let dir = common::tmp();
    let list = [seller(), source("skipme", true)];
    let summaries = all(&list, dir.path(), T2, &make_client(), opts(), 4, None);
    assert_eq!(
        summaries
            .iter()
            .map(|s| s.seller.as_str())
            .collect::<Vec<_>>(),
        ["sellerx"]
    );
    assert_eq!(
        index_sellers(dir.path())
            .iter()
            .map(|s| s.0.as_str())
            .collect::<Vec<_>>(),
        ["sellerx"]
    );
    let summaries = all(
        &list,
        dir.path(),
        T2,
        &make_client(),
        opts(),
        4,
        Some(&["skipme"]),
    );
    assert_eq!(
        summaries
            .iter()
            .map(|s| s.seller.as_str())
            .collect::<Vec<_>>(),
        ["skipme"]
    );

    let dir = common::tmp();
    all(&[seller()], dir.path(), T1, &make_client(), opts(), 4, None);
    all(
        &[source("sellerx", true)],
        dir.path(),
        T2,
        &make_client(),
        opts(),
        4,
        None,
    );
    assert_eq!(
        index_sellers(dir.path()),
        [("sellerx".to_string(), "2".to_string())]
    );
}

#[test]
fn the_default_limiter_keeps_the_politeness_floor() {
    let (global, host, overrides) = lamp::scrape::limiter_settings(12.0);
    assert_eq!((global, host), (12.0, 1.0));
    assert_eq!(overrides, [("photo.yupoo.com", 12.0)]);
}

#[test]
fn several_sellers_are_scraped_and_indexed_once_per_item() {
    let dir = common::tmp();
    let summaries = all(
        &[seller(), source("sellery", false)],
        dir.path(),
        T1,
        &make_client(),
        opts(),
        2,
        None,
    );
    assert_eq!(
        summaries
            .iter()
            .map(|s| s.seller.as_str())
            .collect::<Vec<_>>(),
        ["sellerx", "sellery"]
    );
    assert!(summaries.iter().all(|s| s.scraped == 2));
    assert_eq!(load_shard(dir.path(), "sellery").unwrap().len(), 2);
    let i = index(dir.path());
    let list = i["sellers"].as_array().unwrap();
    let count: i128 = list
        .iter()
        .map(|s| match obj(s)["album_count"] {
            Value::Int(n) => n,
            _ => 0,
        })
        .sum();
    let ids: usize = list
        .iter()
        .map(|s| obj(s)["album_ids"].as_array().unwrap().len())
        .sum();
    assert_eq!((count, ids), (2, 2));
}

#[test]
fn a_corrupt_shard_errors_its_seller_not_the_run() {
    let dir = common::tmp();
    write(&dir.path().join("catalog/sellerx.json"), "{broken");
    let summaries = all(
        &[seller(), source("sellery", false)],
        dir.path(),
        T2,
        &make_client(),
        opts(),
        4,
        None,
    );
    assert!(summaries[0].error.contains("corrupt shard"));
    assert_eq!(summaries[1].scraped, 2);
    assert_eq!(
        index_sellers(dir.path()),
        [
            ("sellerx".to_string(), "0".to_string()),
            ("sellery".to_string(), "2".to_string())
        ]
    );
}

#[test]
fn the_shard_is_flushed_every_hundred_albums() {
    let dir = common::tmp();
    let ids: Vec<String> = (101..251).map(|a| a.to_string()).collect();
    let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
    let client = client_of(&seller_of(&refs));
    let seen = Arc::new(Mutex::new(None));
    let (seen_hook, root) = (seen.clone(), dir.path().to_path_buf());
    *client.0.hook.lock().unwrap() = Some(Arc::new(move |url: &str| {
        if url.contains("/albums/201?") {
            *seen_hook.lock().unwrap() = Some(load_shard(&root, "sellerx").unwrap().len());
        }
    }));
    let summary = run(&client, dir.path(), T1, opts());
    assert_eq!(summary.scraped, 150);
    assert_eq!(*seen.lock().unwrap(), Some(100));
    assert_eq!(shard(dir.path()).len(), 150);
}

#[test]
fn dry_run_previews_without_album_fetches_or_writes() {
    let dir = common::tmp();
    let client = make_client();
    let summary = run(
        &client,
        dir.path(),
        T1,
        Options {
            dry_run: true,
            ..opts()
        },
    );
    assert_eq!(
        (summary.scraped, summary.skipped, summary.failed),
        (2, 0, 0)
    );
    assert!(client.called("albums?page=1"));
    assert!(!client.called("/albums/101") && !client.called("/albums/102"));
    assert!(client.downloads().is_empty());
    assert!(!dir.path().join("catalog/sellerx.json").exists());
    let summaries = all(
        &[seller()],
        dir.path(),
        T1,
        &make_client(),
        Options {
            dry_run: true,
            ..opts()
        },
        4,
        None,
    );
    assert_eq!(summaries[0].scraped, 2);
    assert!(!dir.path().join("index.json").exists());
}

fn pre_price_record(extra: &str) -> Shard {
    let mut record = obj(&json(r#"{"id": "101", "seller": "sellerx", "title": "Item A", "list_title": "Item A", "thumb": "thumbs/sellerx/101.jpg", "album_url": "https://sellerx.x.yupoo.com/albums/101?uid=1", "links": [], "category": "", "photo_count": 3, "first_seen": "2026-08-01T00:00:00+10:00", "updated_at": "2026-08-01T00:00:00+10:00"}"#)).clone();
    if !extra.is_empty() {
        for (k, v) in obj(&json(extra)) {
            record.insert(k.clone(), v.clone());
        }
    }
    [("101".to_string(), record)].into_iter().collect()
}

fn one_album_client(desc: &str) -> FakeClient {
    FakeClient::new(&[
        ("albums?page=1", list_item("101", "Item A")),
        ("albums?page=2", EMPTY_PAGE.into()),
        ("/albums/101", album_page("Item A", desc)),
    ])
}

fn write_thumb(dir: &Path) {
    write(&dir.join("thumbs/sellerx/101.jpg"), b"old cover");
}

#[test]
fn price_backfill_fetches_pre_price_records_but_skips_priced_ones() {
    let dir = common::tmp();
    write_shard(dir.path(), "sellerx", &pre_price_record("")).unwrap();
    let client = one_album_client("Nike Tech Fleece ¥180 Black");
    let summary = run(&client, dir.path(), T2, opts());
    assert_eq!((summary.skipped, summary.scraped), (0, 1));
    assert!(client.called("/albums/101"));
    let s = shard(dir.path());
    assert!(matches!(get(&s, "101", "price"), Value::Int(180)));
    assert_eq!(
        get(&s, "101", "first_seen").py_str(),
        "2026-08-01T00:00:00+10:00"
    );
    let dir = common::tmp();
    write_shard(dir.path(), "sellerx", &pre_price_record(r#"{"price": 0}"#)).unwrap();
    let client = one_album_client("¥180");
    let summary = run(&client, dir.path(), T2, opts());
    assert_eq!((summary.skipped, summary.scraped), (1, 0));
    assert!(!client.called("/albums/101"));
}

#[test]
fn cover_refetch_rules() {
    let dir = common::tmp();
    write_shard(dir.path(), "sellerx", &pre_price_record("")).unwrap();
    write_thumb(dir.path());
    let client = one_album_client("¥180");
    assert_eq!(run(&client, dir.path(), T2, opts()).scraped, 1);
    assert!(client.downloads().is_empty());
    let s = shard(dir.path());
    assert_eq!(
        get(&s, "101", "thumb_url").py_str(),
        "https://photo.yupoo.com/s/101/small.jpg"
    );
    assert!(!s["101"].get("cover_prepared").is_some_and(Value::truthy));

    let dir = common::tmp();
    write_thumb(dir.path());
    write_shard(
        dir.path(),
        "sellerx",
        &pre_price_record(r#"{"thumb_url": "https://photo.yupoo.com/s/101/older.jpg"}"#),
    )
    .unwrap();
    let client = one_album_client("¥180");
    run(&client, dir.path(), T2, opts());
    assert_eq!(client.downloads().len(), 1);
    let client = one_album_client("¥180");
    run(
        &client,
        dir.path(),
        "2026-08-19T12:00:00+10:00",
        Options {
            full: true,
            ..opts()
        },
    );
    assert_eq!(client.downloads().len(), 1);

    let dir = common::tmp();
    write_shard(
        dir.path(),
        "sellerx",
        &pre_price_record(r#"{"price": 0, "thumb": ""}"#),
    )
    .unwrap();
    let client = one_album_client("¥180");
    let summary = run(&client, dir.path(), T2, opts());
    assert_eq!(
        (summary.skipped, summary.scraped, summary.thumbs_refetched),
        (1, 0, 1)
    );
    assert!(!client.called("/albums/101"));
    assert_eq!(client.downloads().len(), 1);
    assert_eq!(
        get(&shard(dir.path()), "101", "thumb").py_str(),
        "thumbs/sellerx/101.jpg"
    );
    let client = one_album_client("¥180");
    let summary = run(&client, dir.path(), "2026-08-19T12:00:00+10:00", opts());
    assert_eq!(summary.thumbs_refetched, 0);
    assert!(client.downloads().is_empty());

    let dir = common::tmp();
    write_thumb(dir.path());
    write_shard(
        dir.path(),
        "sellerx",
        &pre_price_record(
            r#"{"price": 0, "thumb_url": "https://photo.yupoo.com/s/101/older.jpg"}"#,
        ),
    )
    .unwrap();
    let client = one_album_client("¥180");
    let summary = run(&client, dir.path(), T2, opts());
    assert_eq!((summary.skipped, summary.thumbs_refetched), (1, 1));
    assert!(!client.called("/albums/101"));
    assert_eq!(
        get(&shard(dir.path()), "101", "thumb_url").py_str(),
        "https://photo.yupoo.com/s/101/small.jpg"
    );
}

#[test]
fn a_failed_cover_download_keeps_the_old_url_so_it_is_retried() {
    let dir = common::tmp();
    write_thumb(dir.path());
    let older = "https://photo.yupoo.com/s/101/older.jpg";
    write_shard(
        dir.path(),
        "sellerx",
        &pre_price_record(&format!(r#"{{"thumb_url": "{older}"}}"#)),
    )
    .unwrap();
    let client = one_album_client("¥180").block_downloads(&[true]);
    assert_eq!(run(&client, dir.path(), T2, opts()).scraped, 1);
    assert_eq!(get(&shard(dir.path()), "101", "thumb_url").py_str(), older);
    let summary = run(
        &one_album_client("¥180"),
        dir.path(),
        "2026-08-19T12:00:00+10:00",
        opts(),
    );
    assert_eq!(summary.thumbs_refetched, 1);
    assert_ne!(get(&shard(dir.path()), "101", "thumb_url").py_str(), older);
}

#[test]
fn listing_stamps_last_seen_and_marks_delisted_albums_gone() {
    let dir = common::tmp();
    run(&make_client(), dir.path(), T1, opts());
    let only_102 = FakeClient::new(&[
        (
            "albums?page=1",
            list_item("102", "Item B https://item.taobao.com/item.htm?id=7"),
        ),
        ("albums?page=2", EMPTY_PAGE.into()),
    ]);
    run(&only_102, dir.path(), T2, opts());
    let s = shard(dir.path());
    assert_eq!(get(&s, "101", "gone_since").py_str(), T2);
    assert_eq!(get(&s, "101", "last_seen").py_str(), T1);
    assert_eq!(get(&s, "102", "last_seen").py_str(), T2);
    assert!(!s["102"].contains_key("gone_since"));
    all(
        &[seller()],
        dir.path(),
        "2026-08-19T12:00:00+10:00",
        &only_102,
        opts(),
        4,
        None,
    );
    assert_eq!(index_sellers(dir.path())[0].1, "1");
    run(
        &make_client(),
        dir.path(),
        "2026-08-20T12:00:00+10:00",
        opts(),
    );
    assert!(!shard(dir.path())["101"].contains_key("gone_since"));
}

#[test]
fn a_stopped_seller_saves_nothing_it_has_not_listed() {
    let dir = common::tmp();
    let client = make_client();
    let options = opts();
    let summary = scrape_seller(
        &client,
        &seller(),
        dir.path(),
        T2,
        &options,
        &AtomicBool::new(true),
        &RefreshBudget::new(100),
    )
    .unwrap();
    assert_eq!(summary.error, "interrupted before the listing finished");
    assert!(client.calls().is_empty());
    assert!(!dir.path().join("catalog/sellerx.json").exists());
}

#[test]
fn each_seller_is_reported_as_it_finishes() {
    let dir = common::tmp();
    let seen = Mutex::new(Vec::new());
    let interrupt = Interrupt::default();
    let outcome = scrape_all(
        &sources(&[seller(), source("sellery", false)]),
        dir.path(),
        T1,
        &opts(),
        2,
        8.0,
        None,
        Some(make_client().factory()),
        &mut |s| seen.lock().unwrap().push(s.seller.clone()),
        &interrupt,
    )
    .unwrap();
    let mut seen = seen.into_inner().unwrap();
    seen.sort();
    assert_eq!(seen, ["sellerx", "sellery"]);
    assert_eq!(
        outcome
            .summaries
            .iter()
            .map(|s| s.seller.as_str())
            .collect::<Vec<_>>(),
        ["sellerx", "sellery"]
    );
}

#[test]
fn ctrl_c_writes_the_index_then_reports_the_interrupt() {
    let dir = common::tmp();
    let interrupt = Interrupt::default();
    let pressed = Mutex::new(false);
    let outcome = scrape_all(
        &sources(&[seller(), source("sellery", false)]),
        dir.path(),
        T1,
        &opts(),
        1,
        8.0,
        None,
        Some(make_client().factory()),
        &mut |_| {
            let mut pressed = pressed.lock().unwrap();
            if !*pressed {
                *pressed = true;
                interrupt.press();
            }
        },
        &interrupt,
    )
    .unwrap();
    assert!(outcome.interrupted);
    assert!(index_sellers(dir.path()).iter().any(|s| s.0 == "sellerx"));
}

#[test]
fn an_unexpected_error_fails_its_seller_and_the_run_carries_on() {
    let dir = common::tmp();
    let client = make_client();
    *client.0.panic_on.lock().unwrap() = Some("sellery".into());
    let summaries = all(
        &[seller(), source("sellery", false)],
        dir.path(),
        T1,
        &client,
        opts(),
        2,
        None,
    );
    assert_eq!(summaries[0].scraped, 2);
    assert!(
        summaries[1].error.contains("unexpected error")
            && summaries[1].error.contains("parser bug")
    );
    assert!(dir.path().join("index.json").exists());
}

#[test]
fn a_second_ctrl_c_abandons_running_sellers_but_still_writes_the_index() {
    let dir = common::tmp();
    let client = make_client();
    let release = Arc::new(AtomicBool::new(false));
    *client.0.hang_on.lock().unwrap() = Some(("sellery".into(), release.clone()));
    let interrupt = Arc::new(Interrupt::default());
    let presses = Mutex::new(0);
    let started = std::time::Instant::now();
    let second = interrupt.clone();
    let outcome = scrape_all(
        &sources(&[seller(), source("sellery", false)]),
        dir.path(),
        T1,
        &opts(),
        2,
        8.0,
        None,
        Some(client.factory()),
        &mut |_| {
            let mut presses = presses.lock().unwrap();
            if *presses == 0 {
                *presses = 1;
                interrupt.press();
                let second = second.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(300));
                    second.press();
                });
            }
        },
        &interrupt,
    )
    .unwrap();
    assert!(outcome.interrupted);
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
    assert!(dir.path().join("index.json").exists());
    release.store(true, std::sync::atomic::Ordering::SeqCst);
}

#[test]
fn descriptions_are_stored_bounded_after_the_full_text_is_used() {
    let dir = common::tmp();
    let long = format!(
        "{} tail https://weidian.com/item.html?itemID=9090",
        "x".repeat(500)
    );
    let client = FakeClient::new(&[
        ("albums?page=1", list_item("301", "Item C")),
        ("albums?page=2", EMPTY_PAGE.into()),
        ("/albums/301", album_page("Item C", &long)),
    ]);
    run(&client, dir.path(), T1, opts());
    let s = shard(dir.path());
    assert_eq!(get(&s, "301", "description").py_str(), "x".repeat(500));
    assert!(matches!(
        get(&s, "301", "description_truncated"),
        Value::Bool(true)
    ));
    assert_eq!(
        get(&s, "301", "links").py_str(),
        r#"[{"platform": "weidian", "url": "https://weidian.com/item.html?itemID=9090"}]"#
    );
    let plain = dir.path().join("plain");
    run(&make_client(), &plain, T1, opts());
    let p = load_shard(&plain, "sellerx").unwrap();
    assert_eq!(
        get(&p, "101", "description").py_str(),
        "weidian.com/item.html?itemID=5"
    );
    assert!(!p["101"].contains_key("description_truncated"));
}

#[test]
fn scrape_lock_precedes_any_fetch_and_dry_runs_create_nothing() {
    let dir = common::tmp();
    let _lock = lamp::locking::CatalogueLock::acquire(dir.path()).unwrap();
    let interrupt = Interrupt::default();
    let result = scrape_all(
        &Sources::new(),
        dir.path(),
        "2026-09-26T00:00:00+00:00",
        &opts(),
        4,
        8.0,
        None,
        None,
        &mut |_| {},
        &interrupt,
    );
    assert!(matches!(result, Err(lamp::errors::Error::Busy(_))));
    let target = dir.path().join("absent");
    let outcome = scrape_all(
        &Sources::new(),
        &target,
        "2026-09-26T00:00:00+00:00",
        &Options {
            dry_run: true,
            ..opts()
        },
        4,
        8.0,
        None,
        None,
        &mut |_| {},
        &interrupt,
    )
    .unwrap();
    assert!(outcome.summaries.is_empty());
    assert!(!target.exists());
}
