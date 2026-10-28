//! Scrape Yupoo sellers into catalogue shards.

use std::collections::{BTreeSet, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use crate::age::is_stale;
use crate::covers::cover_facts;
use crate::errors::{Error, Result};
use crate::http::{self, Client, FetchError, RateLimiter};
use crate::ingest::{Source, Sources};
use crate::isodate;
use crate::links::extract_links;
use crate::locking::CatalogueLock;
use crate::parse::{AlbumStub, parse_album_list, parse_album_page, price_evidence};
use crate::pyerr;
use crate::pyjson::{Map, Value, py_eq};
use crate::pystr;
use crate::storage::{
    Shard, load_shard, merge_album, read_run, record_change, write_index, write_run, write_shard,
};

/// Stored album text: enough for product codes, sizes and batch names.
pub const DESCRIPTION_LIMIT: usize = 500;
const BLOCKED_STREAK_LIMIT: u32 = 3;
const THUMB_BLOCKED_STREAK_LIMIT: u32 = 3;
/// Distinct unparsed album links before reporting ambiguous listing coverage.
const LAYOUT_CHANGE_MIN_LINKS: usize = 3;
/// A listing that drops this many listed albums at once marks none gone until
/// the next run also drops at least this many. shortcut: a fixed count, not a
/// page-aware check; it misses a truncated listing that drops fewer albums
/// (those come back when listed again). Upgrade if Yupoo exposes a page or
/// album total to compare against.
const SHORT_LISTING_MIN_LOST: usize = 5;
const FLUSH_EVERY: usize = 100;

/// What a scraper needs from an HTTP client; tests substitute fakes.
pub trait ScrapeClient: Send {
    fn get(&self, url: &str, referer: Option<&str>) -> std::result::Result<String, FetchError>;
    fn download(
        &self,
        url: &str,
        dest: &Path,
        referer: Option<&str>,
    ) -> std::result::Result<bool, FetchError>;
}

impl ScrapeClient for Client {
    fn get(&self, url: &str, referer: Option<&str>) -> std::result::Result<String, FetchError> {
        Client::get(self, url, referer).map(|page| page.text())
    }

    fn download(
        &self,
        url: &str,
        dest: &Path,
        referer: Option<&str>,
    ) -> std::result::Result<bool, FetchError> {
        Client::download(self, url, dest, referer)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SellerSummary {
    pub seller: String,
    pub scraped: i64,
    pub skipped: i64,
    pub failed: i64,
    pub error: String,
    pub failed_ids: Vec<String>,
    pub thumbs_blocked: bool,
    pub dated: i64,
    pub skipped_old: i64,
    pub thumbs_refetched: i64,
    pub refreshed: i64,
    pub deferred: i64,
    pub listing_complete: bool,
    pub resolved_ids: Vec<String>,
    pub short_listing: i64,
}

impl SellerSummary {
    pub fn new(seller: &str) -> SellerSummary {
        SellerSummary {
            seller: seller.into(),
            ..Default::default()
        }
    }

    /// `dataclasses.asdict`.
    pub fn as_map(&self) -> Map {
        let strings =
            |items: &[String]| Value::Array(items.iter().map(|s| Value::Str(s.clone())).collect());
        crate::obj! {
            "seller" => self.seller.as_str(),
            "scraped" => self.scraped,
            "skipped" => self.skipped,
            "failed" => self.failed,
            "error" => self.error.as_str(),
            "failed_ids" => strings(&self.failed_ids),
            "thumbs_blocked" => self.thumbs_blocked,
            "dated" => self.dated,
            "skipped_old" => self.skipped_old,
            "thumbs_refetched" => self.thumbs_refetched,
            "refreshed" => self.refreshed,
            "deferred" => self.deferred,
            "listing_complete" => self.listing_complete,
            "resolved_ids" => strings(&self.resolved_ids),
            "short_listing" => self.short_listing,
        }
    }
}

/// Unchanged-detail refreshes shared across one run.
pub struct RefreshBudget {
    remaining: Mutex<i128>,
}

impl RefreshBudget {
    pub fn new(limit: i128) -> RefreshBudget {
        RefreshBudget {
            remaining: Mutex::new(limit),
        }
    }

    pub fn take(&self) -> bool {
        let mut remaining = self.remaining.lock().unwrap_or_else(|e| e.into_inner());
        if *remaining <= 0 {
            return false;
        }
        *remaining -= 1;
        true
    }
}

fn utc_micros(text: &str) -> Option<i64> {
    let (local, offset) = isodate::parse_datetime(text)?;
    Some(local - offset.unwrap_or(0))
}

pub fn detail_due(album: &Map, run_ts: &str, days: i128) -> bool {
    if days <= 0 {
        return false;
    }
    let previous = match album.get("updated_at") {
        None => utc_micros(""),
        Some(Value::Str(s)) => utc_micros(s),
        Some(_) => None,
    };
    match (previous, utc_micros(run_ts)) {
        (Some(previous), Some(now)) => {
            i128::from(now - previous) >= days.saturating_mul(86_400).saturating_mul(1_000_000)
        }
        _ => true,
    }
}

/// Run-wide options, as the CLI passes them.
#[derive(Clone, Debug)]
pub struct Options {
    pub full: bool,
    pub max_age_days: i128,
    pub dry_run: bool,
    pub refresh_days: i128,
    pub refresh_limit: i128,
    pub retry_failed: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            full: false,
            max_age_days: 0,
            dry_run: false,
            refresh_days: 0,
            refresh_limit: 100,
            retry_failed: false,
        }
    }
}

enum ListingError {
    Stopped,
    Message(String),
}

/// `re.findall(r"/albums/\d+", html)` as a set.
fn album_links(html: &str) -> HashSet<String> {
    let mut links = HashSet::new();
    let mut rest = html;
    while let Some(pos) = rest.find("/albums/") {
        let after = &rest[pos + 8..];
        let digits: String = after
            .chars()
            .take_while(|&c| pystr::decimal_value(c).is_some())
            .collect();
        if digits.is_empty() {
            rest = &rest[pos + 1..];
        } else {
            rest = &after[digits.len()..];
            links.insert(format!("/albums/{digits}"));
        }
    }
    links
}

fn fetch_all_stubs(
    client: &dyn ScrapeClient,
    source: &Source,
    data_dir: &Path,
    save_debug: bool,
    stop: &AtomicBool,
) -> std::result::Result<Vec<AlbumStub>, ListingError> {
    let base = source.url_str();
    let mut stubs: Vec<AlbumStub> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut page = 1u64;
    loop {
        if stop.load(Ordering::SeqCst) {
            return Err(ListingError::Stopped);
        }
        let url = format!("{base}/albums?page={page}");
        let html = client.get(&url, Some(&base)).map_err(|e| match e {
            FetchError::Blocked(_) => ListingError::Message(format!("blocked: {e}")),
            FetchError::Fetch(m) => ListingError::Message(m),
        })?;
        let batch = parse_album_list(&html);
        let listed: HashSet<String> = batch.iter().map(|s| format!("/albums/{}", s.id)).collect();
        let unreadable = album_links(&html).difference(&listed).count();
        let ambiguous = unreadable >= LAYOUT_CHANGE_MIN_LINKS;
        let mut saved_note = String::new();
        if save_debug && (ambiguous || (batch.is_empty() && page == 1)) {
            let debug = data_dir
                .join("debug")
                .join(format!("{}-page{page}.html", source.id));
            if let Some(parent) = debug.parent() {
                let _ = pyerr::mkdir_parents(parent);
            }
            let _ = std::fs::write(&debug, pyerr::native_newlines(&html));
            saved_note = format!("; saved {}", pyerr::path_str(&debug));
        }
        if ambiguous {
            return Err(ListingError::Message(format!(
                "page {page} of {} has {unreadable} album links that could not be parsed{saved_note} - possible Yupoo layout change?",
                source.id
            )));
        }
        if batch.is_empty() {
            if page == 1 {
                return Err(ListingError::Message(format!(
                    "page 1 of {} contains no albums{saved_note} - empty or closed seller?",
                    source.id
                )));
            }
            return Ok(stubs);
        }
        let new: Vec<AlbumStub> = batch
            .into_iter()
            .filter(|s| !seen.contains(&s.id))
            .collect();
        if new.is_empty() {
            return Ok(stubs);
        }
        for stub in &new {
            seen.insert(stub.id.clone());
        }
        stubs.extend(new);
        page += 1;
    }
}

fn get_str<'a>(map: Option<&'a Map>, key: &str) -> Option<&'a Value> {
    map.and_then(|m| m.get(key))
}

#[allow(clippy::too_many_arguments)]
pub fn scrape_seller(
    client: &dyn ScrapeClient,
    source: &Source,
    data_dir: &Path,
    run_ts: &str,
    options: &Options,
    stop: &AtomicBool,
    budget: &RefreshBudget,
) -> Result<SellerSummary> {
    let mut summary = SellerSummary::new(&source.id);
    let loaded = load_shard(data_dir, &source.id).and_then(|shard| {
        let run = read_run(data_dir, &source.id)?;
        let failures: BTreeSet<String> = match run.get("failed_ids") {
            Some(Value::Array(ids)) => ids.iter().map(Value::py_str).collect(),
            _ => BTreeSet::new(),
        };
        let short_before = run.get("short_listing").is_some_and(Value::truthy);
        Ok((shard, failures, short_before))
    });
    let (mut shard, pending_failures, short_before): (Shard, BTreeSet<String>, bool) = match loaded
    {
        Ok(found) => found,
        Err(Error::Value(message)) => {
            summary.error = message;
            return Ok(summary);
        }
        Err(other) => return Err(other),
    };
    let today = isodate::parse_date(pystr::prefix_chars(run_ts, 10)).ok_or_else(|| {
        Error::Value(format!(
            "Invalid isoformat string: {}",
            pystr::repr(pystr::prefix_chars(run_ts, 10))
        ))
    })?;
    let base = source.url_str();
    let mut blocked_streak = 0;
    let mut thumb_blocked_streak = 0;
    let mut thumbs_disabled = false;

    let stubs = match fetch_all_stubs(client, source, data_dir, !options.dry_run, stop) {
        Ok(stubs) => stubs,
        Err(ListingError::Stopped) => {
            summary.error = "interrupted before the listing finished".into();
            return Ok(summary);
        }
        Err(ListingError::Message(message)) => {
            summary.error = message;
            return Ok(summary);
        }
    };
    let listed: HashSet<String> = stubs.iter().map(|s| s.id.clone()).collect();
    let lost = shard
        .iter()
        .filter(|(id, album)| {
            !listed.contains(*id) && !album.get("gone_since").is_some_and(Value::truthy)
        })
        .count();
    if lost >= SHORT_LISTING_MIN_LOST && !short_before {
        summary.short_listing = stubs.len() as i64;
        summary.error = format!(
            "listing ended {lost} albums short of the last one; they are not marked gone until another run confirms it - possible block or layout change?"
        );
    } else {
        summary.listing_complete = true;
        summary.resolved_ids.extend(
            pending_failures
                .iter()
                .filter(|id| !listed.contains(*id))
                .cloned(),
        );
    }
    let mut stubs = stubs;
    if options.refresh_days != 0 {
        stubs.sort_by_key(
            |stub| match shard.get(&stub.id).and_then(|a| a.get("updated_at")) {
                Some(Value::Str(s)) => s.clone(),
                _ => String::new(),
            },
        );
    }
    let mut scraped_since_flush = 0;
    for stub in &stubs {
        if stop.load(Ordering::SeqCst) {
            summary.error = "interrupted; progress so far is saved".into();
            break;
        }
        let old = shard.get(&stub.id).cloned();
        let old_ref = old.as_ref();
        if options.retry_failed && !pending_failures.contains(&stub.id) {
            summary.skipped += 1;
            continue;
        }
        let thumb_rel = format!("thumbs/{}/{}.jpg", source.id, stub.id);
        let thumb_path = pyerr::join_rel(data_dir, &thumb_rel);
        let held_url = get_str(old_ref, "thumb_url")
            .cloned()
            .unwrap_or_else(|| stub.thumb_url.as_str().into());
        let mut cover_current = stub.thumb_url.is_empty()
            || (py_eq(&held_url, &Value::Str(stub.thumb_url.clone())) && thumb_path.exists());
        let known = match get_str(old_ref, "date_published") {
            Some(Value::Str(s)) => s.as_str(),
            _ => "",
        };
        let numeric_id = pystr::parse_int(&stub.id).map_or(u128::MAX, |n| n.max(0) as u128);
        if is_stale(numeric_id, options.max_age_days, today, known) {
            summary.skipped_old += 1;
            continue;
        }
        let unchanged = old_ref.is_some_and(|o| {
            o.get("list_title")
                .is_some_and(|t| py_eq(t, &Value::Str(stub.title.clone())))
                && o.contains_key("price")
        });
        let mut refresh = false;
        if let Some(o) = old_ref
            && unchanged
            && !options.full
            && !options.retry_failed
            && detail_due(o, run_ts, options.refresh_days)
        {
            refresh = budget.take();
            if !refresh {
                summary.deferred += 1;
            }
        }
        let mut fetch_thumb = |url: &str, rel_path: &Path, summary: &mut SellerSummary| -> bool {
            if thumbs_disabled {
                return false;
            }
            match client.download(url, rel_path, Some(&base)) {
                Ok(fetched) => {
                    thumb_blocked_streak = 0;
                    fetched
                }
                Err(FetchError::Blocked(_)) => {
                    thumb_blocked_streak += 1;
                    if thumb_blocked_streak >= THUMB_BLOCKED_STREAK_LIMIT {
                        thumbs_disabled = true;
                        summary.thumbs_blocked = true;
                    }
                    false
                }
                Err(FetchError::Fetch(_)) => false,
            }
        };
        if let Some(o) = old_ref
            && unchanged
            && !options.full
            && !options.retry_failed
            && !refresh
        {
            summary.skipped += 1;
            if !options.dry_run
                && !cover_current
                && fetch_thumb(&stub.thumb_url, &thumb_path, &mut summary)
            {
                summary.thumbs_refetched += 1;
                let mut updated = o.clone();
                updated.insert("thumb".into(), thumb_rel.clone().into());
                updated.insert("thumb_url".into(), stub.thumb_url.clone().into());
                for (k, v) in cover_facts(&thumb_path) {
                    updated.insert(k, v);
                }
                updated.insert("cover_prepared".into(), Value::Int(1));
                shard.insert(stub.id.clone(), updated);
            }
            continue;
        }
        if options.dry_run {
            summary.refreshed += i64::from(refresh);
            summary.scraped += 1;
            continue;
        }
        let album_url = format!("{base}/albums/{}?uid=1", stub.id);
        let detail = match client.get(&album_url, Some(&base)) {
            Ok(text) => {
                blocked_streak = 0;
                parse_album_page(&text)
            }
            Err(FetchError::Blocked(status)) => {
                blocked_streak += 1;
                summary.failed += 1;
                summary.failed_ids.push(stub.id.clone());
                if blocked_streak >= BLOCKED_STREAK_LIMIT {
                    summary.error = format!(
                        "blocked: aborted after {blocked_streak} consecutive ({})",
                        FetchError::Blocked(status)
                    );
                    break;
                }
                continue;
            }
            Err(FetchError::Fetch(_)) => {
                summary.failed += 1;
                summary.failed_ids.push(stub.id.clone());
                continue;
            }
        };
        if detail.title.is_empty() && detail.description.is_empty() {
            summary.failed += 1;
            summary.failed_ids.push(stub.id.clone());
            continue;
        }
        let title = if detail.title.is_empty() {
            stub.title.clone()
        } else {
            detail.title.clone()
        };
        let links = extract_links(&format!("{title}\n{}\n{}", stub.title, detail.description));
        let mut evidence = price_evidence(&format!("{title} {}", detail.description));
        if evidence.is_empty() {
            evidence = price_evidence(&format!("{title} {} {}", stub.title, detail.description));
        }
        if !evidence.is_empty() {
            let token = evidence.get("token").map(Value::py_str).unwrap_or_default();
            let field = [
                ("title", &title),
                ("listing title", &stub.title),
                ("description", &detail.description),
            ]
            .into_iter()
            .find(|(_, text)| text.contains(token.as_str()))
            .map_or("album text", |(name, _)| name);
            evidence.insert("field".into(), field.into());
        }
        let price = evidence.get("amount").cloned().unwrap_or(Value::Int(0));
        let mut cover_downloaded = false;
        if !stub.thumb_url.is_empty() && (options.full || !cover_current) {
            cover_current = fetch_thumb(&stub.thumb_url, &thumb_path, &mut summary);
            cover_downloaded = cover_current;
        }
        let thumb_url_value: Value = if cover_current {
            stub.thumb_url.clone().into()
        } else {
            get_str(old_ref, "thumb_url")
                .cloned()
                .unwrap_or_else(|| "".into())
        };
        let link_values: Vec<Value> = links
            .iter()
            .map(|l| {
                Value::Object(
                    crate::obj! { "platform" => l.platform.as_str(), "url" => l.url.as_str() },
                )
            })
            .collect();
        let photo_count = if detail.photo_count != 0 {
            detail.photo_count
        } else {
            stub.photo_count
        };
        let mut record = crate::obj! {
            "id" => stub.id.as_str(),
            "seller" => source.id.as_str(),
            "title" => title.as_str(),
            "list_title" => stub.title.as_str(),
            "thumb" => if stub.thumb_url.is_empty() { "" } else { thumb_rel.as_str() },
            "thumb_url" => thumb_url_value,
            "album_url" => album_url.as_str(),
            "links" => link_values,
            "category" => detail.category.as_str(),
            "description" => pystr::prefix_chars(&detail.description, DESCRIPTION_LIMIT),
            "price" => price,
            "price_evidence" => Value::Object(evidence),
            "photo_count" => Value::Int(photo_count),
            "date_published" => detail.date_published.as_str(),
            "date_modified" => detail.date_modified.as_str(),
            "first_seen" => "",
            "updated_at" => "",
        };
        if pystr::char_len(&detail.description) > DESCRIPTION_LIMIT {
            record.insert("description_truncated".into(), true.into());
        }
        if !stub.thumb_url.is_empty() && cover_current && thumb_path.is_file() {
            for (k, v) in cover_facts(&thumb_path) {
                record.insert(k, v);
            }
            if cover_downloaded {
                record.insert("cover_prepared".into(), Value::Int(1));
            }
        }
        let merged = merge_album(old_ref, &record, run_ts);
        shard.insert(stub.id.clone(), merged);
        summary.scraped += 1;
        summary.refreshed += i64::from(refresh);
        summary.resolved_ids.push(stub.id.clone());
        if !detail.date_published.is_empty() || !detail.date_modified.is_empty() {
            summary.dated += 1;
        }
        scraped_since_flush += 1;
        if scraped_since_flush >= FLUSH_EVERY {
            write_shard(data_dir, &source.id, &shard)?;
            scraped_since_flush = 0;
        }
    }
    let attempted = stubs.len() as i64 - summary.skipped;
    if attempted > 0 && summary.scraped == 0 && summary.failed > 0 && summary.error.is_empty() {
        summary.error = format!("all {} album fetches failed", summary.failed);
    }
    if !options.dry_run {
        stamp_listing(&mut shard, &listed, run_ts, summary.listing_complete);
        write_shard(data_dir, &source.id, &shard)?;
    }
    Ok(summary)
}

/// Record which albums the seller's listing still carries; only a complete
/// listing marks the others gone.
fn stamp_listing(shard: &mut Shard, listed: &HashSet<String>, run_ts: &str, complete: bool) {
    for (album_id, album) in shard.iter_mut() {
        if listed.contains(album_id) {
            album.insert("last_seen".into(), run_ts.into());
            let gone = album.shift_remove("gone_since");
            if gone.is_some_and(|g| g.truthy()) {
                record_change(
                    album,
                    "availability",
                    "gone".into(),
                    "listed".into(),
                    run_ts,
                );
            }
        } else if complete && !album.get("gone_since").is_some_and(Value::truthy) {
            record_change(
                album,
                "availability",
                "listed".into(),
                "gone".into(),
                run_ts,
            );
            album.insert("gone_since".into(), run_ts.into());
        }
    }
}

/// Ctrl+C presses, counted. The first stops queued sellers and lets running
/// ones save; the second abandons them.
#[derive(Default)]
pub struct Interrupt {
    presses: AtomicUsize,
}

impl Interrupt {
    pub fn press(&self) {
        self.presses.fetch_add(1, Ordering::SeqCst);
    }

    pub fn presses(&self) -> usize {
        self.presses.load(Ordering::SeqCst)
    }
}

pub type ClientFactory = Arc<dyn Fn() -> Box<dyn ScrapeClient> + Send + Sync>;

/// What `scrape_all` returns: summaries, and whether the run was interrupted
/// (Python raises KeyboardInterrupt after writing the index).
pub struct RunOutcome {
    pub summaries: Vec<SellerSummary>,
    pub interrupted: bool,
}

fn panic_text(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "panic".into()
    }
}

/// The default limiter: the operator's global cap, a fixed 1 request per
/// second per seller host as a politeness floor, and the shared cover CDN
/// at the global cap (it would otherwise serialise the whole run).
pub fn limiter_settings(rps: f64) -> (f64, f64, Vec<(&'static str, f64)>) {
    (rps, 1.0, vec![("photo.yupoo.com", rps)])
}

/// Scrape every seller, then rewrite the index from the shards on disk.
#[allow(clippy::too_many_arguments)]
pub fn scrape_all(
    sources: &Sources,
    data_dir: &Path,
    run_ts: &str,
    options: &Options,
    workers: i128,
    rps: f64,
    only: Option<&HashSet<String>>,
    client_factory: Option<ClientFactory>,
    on_done: &mut dyn FnMut(&SellerSummary),
    interrupt: &Interrupt,
) -> Result<RunOutcome> {
    let budget = Arc::new(RefreshBudget::new(options.refresh_limit));
    let lock = if options.dry_run {
        None
    } else {
        Some(CatalogueLock::acquire(data_dir)?)
    };
    let factory: ClientFactory = client_factory.unwrap_or_else(|| {
        let (global, host, overrides) = limiter_settings(rps);
        let limiter = Arc::new(RateLimiter::new(
            global,
            host,
            &overrides,
            Arc::new(http::SystemClock::default()),
        ));
        Arc::new(move || Box::new(Client::new(limiter.clone())) as Box<dyn ScrapeClient>)
    });
    let mut sellers: Vec<&Source> = sources.values().filter(|s| s.kind == "yupoo").collect();
    sellers.sort_by(|a, b| a.id.cmp(&b.id));
    let mut shops: Vec<&Source> = sources.values().filter(|s| s.kind == "shop").collect();
    shops.sort_by(|a, b| a.id.cmp(&b.id));
    let to_scrape: Vec<Source> = sellers
        .iter()
        .filter(|s| match only {
            Some(only) => only.contains(&s.id),
            None => !s.skip,
        })
        .map(|s| (*s).clone())
        .collect();

    let stop = Arc::new(AtomicBool::new(false));
    let queue = Arc::new(Mutex::new(std::collections::VecDeque::from(
        to_scrape.clone(),
    )));
    let (sender, receiver) = mpsc::channel::<SellerSummary>();
    let thread_count = usize::try_from(workers.max(0))
        .unwrap_or(usize::MAX)
        .min(to_scrape.len());
    let mut handles = Vec::new();
    for _ in 0..thread_count {
        let (queue, sender, stop, factory, budget) = (
            queue.clone(),
            sender.clone(),
            stop.clone(),
            factory.clone(),
            budget.clone(),
        );
        let data_dir = data_dir.to_path_buf();
        let run_ts = run_ts.to_string();
        let options = options.clone();
        handles.push(std::thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                let Some(source) = queue.lock().unwrap_or_else(|e| e.into_inner()).pop_front()
                else {
                    return;
                };
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let client = factory();
                    scrape_seller(
                        &*client, &source, &data_dir, &run_ts, &options, &stop, &budget,
                    )
                }));
                let summary = match outcome {
                    Ok(Ok(summary)) => summary,
                    Ok(Err(error)) => {
                        eprint_unexpected(&source.id, &error.to_string());
                        SellerSummary {
                            error: format!("unexpected error: {error}"),
                            ..SellerSummary::new(&source.id)
                        }
                    }
                    Err(payload) => {
                        let text = panic_text(&*payload);
                        eprint_unexpected(&source.id, &text);
                        SellerSummary {
                            error: format!("unexpected error: {text}"),
                            ..SellerSummary::new(&source.id)
                        }
                    }
                };
                if sender.send(summary).is_err() {
                    return;
                }
            }
        }));
    }
    drop(sender);

    let mut summaries: Vec<SellerSummary> = Vec::new();
    let mut announced = false;
    let mut drain_error: Option<Error> = None;
    loop {
        let presses = interrupt.presses();
        if presses >= 1 && !announced {
            announced = true;
            stop.store(true, Ordering::SeqCst);
            crate::out::err(
                "stopping: running sellers save their progress, Ctrl+C again to abandon them",
            );
        }
        if presses >= 2 {
            break;
        }
        match receiver.recv_timeout(Duration::from_millis(500)) {
            Ok(item) => {
                if !options.dry_run
                    && let Err(e) = write_run(data_dir, &item.seller, run_ts, item.as_map())
                {
                    drain_error = Some(e);
                    break;
                }
                on_done(&item);
                summaries.push(item);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    stop.store(true, Ordering::SeqCst);
    let interrupted = announced;
    // Workers still running after a second Ctrl+C keep the lock until they
    // finish or the process exits.
    let release = move || {
        for handle in handles {
            let _ = handle.join();
        }
        drop(lock);
    };
    let abandoned = interrupt.presses() >= 2 || drain_error.is_some();
    let mut release = Some(release);
    let mut finish = || {
        if let Some(release) = release.take() {
            if abandoned {
                std::thread::spawn(release);
            } else {
                release();
            }
        }
    };
    if let Some(error) = drain_error {
        finish();
        return Err(error);
    }
    summaries.sort_by(|a, b| a.seller.cmp(&b.seller));
    if !options.dry_run
        && let Err(error) = write_scrape_index(data_dir, &sellers, &shops, run_ts)
    {
        finish();
        return Err(error);
    }
    finish();
    Ok(RunOutcome {
        summaries,
        interrupted,
    })
}

fn eprint_unexpected(seller: &str, text: &str) {
    crate::out::err_from_worker(&format!("{seller}: unexpected error\n{text}"));
}

/// The index covers every yupoo source except skipped ones with no shard.
pub fn write_scrape_index(
    data_dir: &Path,
    sellers: &[&Source],
    shops: &[&Source],
    run_ts: &str,
) -> Result<()> {
    let mut seller_meta = Vec::new();
    for source in sellers {
        let album_count = match load_shard(data_dir, &source.id) {
            Err(Error::Value(_)) => 0,
            Err(other) => return Err(other),
            Ok(shard) => {
                if source.skip && shard.is_empty() {
                    continue;
                }
                shard
                    .values()
                    .filter(|a| !a.get("gone_since").is_some_and(Value::truthy))
                    .count()
            }
        };
        seller_meta.push(crate::obj! {
            "id" => source.id.as_str(),
            "name" => source.name.clone(),
            "url" => source.url.clone(),
            "album_count" => album_count,
            "shard" => format!("catalog/{}.json", source.id),
            "run" => Value::Object(read_run(data_dir, &source.id)?),
        });
    }
    let shop_meta: Vec<Value> = shops
        .iter()
        .map(|s| {
            Value::Object(crate::obj! {
                "id" => s.id.as_str(),
                "platform" => s.platform.clone(),
                "name" => s.name.clone(),
                "url" => s.url.clone(),
            })
        })
        .collect();
    write_index(
        data_dir,
        &seller_meta,
        &Value::Array(shop_meta),
        run_ts,
        false,
    )
}
