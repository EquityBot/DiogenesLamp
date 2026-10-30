//! Ported from tests/test_http.py and tests/test_safe_fetch.py.
mod common;

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use lamp::http::{
    self, Client, Clock, FetchError, Headers, Lru, MAX_IMAGE_BYTES, MAX_REDIRECTS,
    MAX_RETRY_AFTER_SECS, RETRIES, RateLimiter, Response, Target, TransportError, normalise_url,
    open_each_address,
};

#[derive(Default)]
struct FakeClock {
    state: Mutex<(f64, Vec<f64>)>,
}

impl Clock for FakeClock {
    fn now(&self) -> f64 {
        self.state.lock().unwrap().0
    }
    fn sleep(&self, seconds: f64) {
        let mut s = self.state.lock().unwrap();
        s.1.push(seconds);
        s.0 += seconds;
    }
}

impl FakeClock {
    fn sleeps(&self) -> Vec<f64> {
        self.state.lock().unwrap().1.clone()
    }
}

#[derive(Default)]
struct Flag(Mutex<bool>, Condvar);

impl Flag {
    fn set(&self) {
        *self.0.lock().unwrap() = true;
        self.1.notify_all();
    }
    fn wait(&self) -> bool {
        let guard = self.0.lock().unwrap();
        let (guard, _) = self
            .1
            .wait_timeout_while(guard, std::time::Duration::from_secs(5), |set| !*set)
            .unwrap();
        *guard
    }
}

/// Blocks one sleeper while other workers advance the shared clock.
#[derive(Default)]
struct GatedClock {
    now: Mutex<f64>,
    sleeps: Mutex<Vec<f64>>,
    gate_next: AtomicBool,
    reserved: Flag,
    release: Flag,
}

impl Clock for GatedClock {
    fn now(&self) -> f64 {
        *self.now.lock().unwrap()
    }
    fn sleep(&self, seconds: f64) {
        let deadline = *self.now.lock().unwrap() + seconds;
        if self.gate_next.swap(false, Ordering::SeqCst) {
            self.reserved.set();
            assert!(self.release.wait(), "test deadlocked waiting for release");
        }
        self.sleeps.lock().unwrap().push(seconds);
        let mut now = self.now.lock().unwrap();
        *now = now.max(deadline);
    }
}

fn approx(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

#[test]
fn rate_limiter_enforces_global_and_host_gaps() {
    let clock = Arc::new(FakeClock::default());
    let limiter = RateLimiter::new(4.0, 1.0, &[], clock.clone());
    limiter.wait("a.example");
    limiter.wait("b.example");
    assert!(approx(clock.now(), 0.25));
    limiter.wait("a.example");
    assert!(approx(clock.now(), 1.0));
}

#[test]
fn rate_limiter_host_override_uses_global_gap() {
    let clock = Arc::new(FakeClock::default());
    let limiter = RateLimiter::new(4.0, 1.0, &[("photo.example", 4.0)], clock.clone());
    limiter.wait("photo.example");
    limiter.wait("photo.example");
    assert!(approx(clock.now(), 0.25));
}

#[test]
fn host_delay_does_not_drag_back_the_global_slot() {
    let clock = Arc::new(GatedClock::default());
    let limiter = Arc::new(RateLimiter::new(4.0, 1.0, &[], clock.clone()));
    limiter.wait("a");
    clock.gate_next.store(true, Ordering::SeqCst);
    let worker = {
        let limiter = limiter.clone();
        std::thread::spawn(move || limiter.wait("a"))
    };
    assert!(clock.reserved.wait(), "worker never reached its sleep");
    limiter.wait("b");
    assert!(approx(clock.now(), 0.25));
    assert_eq!(clock.sleeps.lock().unwrap().len(), 1);
    clock.release.set();
    worker.join().unwrap();
    assert!(approx(clock.now(), 1.0));
    let sleeps = clock.sleeps.lock().unwrap().clone();
    assert!(approx(sleeps[0], 0.25) && approx(sleeps[1], 1.0));
}

#[test]
fn delayed_host_rechecks_the_global_gap_when_other_hosts_overtake() {
    let clock = Arc::new(GatedClock::default());
    let limiter = Arc::new(RateLimiter::new(4.0, 1.0, &[], clock.clone()));
    limiter.wait("a");
    clock.gate_next.store(true, Ordering::SeqCst);
    let admitted = Arc::new(Mutex::new(Vec::new()));
    let worker = {
        let (limiter, clock, admitted) = (limiter.clone(), clock.clone(), admitted.clone());
        std::thread::spawn(move || {
            limiter.wait("a");
            admitted.lock().unwrap().push(clock.now());
        })
    };
    assert!(clock.reserved.wait());
    *clock.now.lock().unwrap() = 1.0;
    limiter.wait("b");
    clock.release.set();
    worker.join().unwrap();
    assert_eq!(admitted.lock().unwrap().len(), 1);
    assert!(approx(admitted.lock().unwrap()[0], 1.25));
}

struct FakeResponse {
    status: u16,
    headers: Vec<(String, String)>,
    chunks: VecDeque<Result<Vec<u8>, String>>,
    closed: Arc<AtomicBool>,
}

impl FakeResponse {
    fn new(status: u16, body: &[u8], headers: &[(&str, &str)]) -> FakeResponse {
        FakeResponse {
            status,
            headers: headers
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            chunks: VecDeque::from([Ok(body.to_vec())]),
            closed: Arc::new(AtomicBool::new(false)),
        }
    }

    fn chunks(mut self, chunks: Vec<Result<Vec<u8>, String>>) -> FakeResponse {
        self.chunks = chunks.into();
        self
    }
}

impl Drop for FakeResponse {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::SeqCst);
    }
}

impl Response for FakeResponse {
    fn status(&self) -> u16 {
        self.status
    }
    fn header(&self, name: &str) -> Option<String> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.clone())
    }
    fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, String> {
        match self.chunks.pop_front() {
            None => Ok(None),
            Some(Ok(c)) => Ok(Some(c)),
            Some(Err(e)) => Err(e),
        }
    }
}

type Calls = Arc<Mutex<Vec<(Target, Headers)>>>;

fn make_client(
    resolver: impl Fn(&str, u16) -> Result<Vec<String>, String> + Send + Sync + 'static,
    responses: Vec<Result<FakeResponse, TransportError>>,
) -> (Client, Calls, Arc<FakeClock>) {
    let clock = Arc::new(FakeClock::default());
    let calls: Calls = Arc::default();
    let queue = Arc::new(Mutex::new(VecDeque::from(responses)));
    let recorded = calls.clone();
    let transport =
        move |target: &Target, headers: &Headers| -> Result<Box<dyn Response>, TransportError> {
            recorded
                .lock()
                .unwrap()
                .push((target.clone(), headers.clone()));
            match queue
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected request")
            {
                Ok(response) => Ok(Box::new(response)),
                Err(e) => Err(e),
            }
        };
    let limiter = Arc::new(RateLimiter::new(4.0, 1.0, &[], clock.clone()));
    (
        Client::with_parts(
            limiter,
            Arc::new(resolver),
            Arc::new(transport),
            clock.clone(),
        ),
        calls,
        clock,
    )
}

fn global(_: &str, _: u16) -> Result<Vec<String>, String> {
    Ok(vec!["8.8.8.8".into()])
}

fn static_answers(
    answers: &[&str],
) -> impl Fn(&str, u16) -> Result<Vec<String>, String> + Send + Sync + 'static {
    let answers: Vec<String> = answers.iter().map(|s| s.to_string()).collect();
    move |_, _| Ok(answers.clone())
}

fn header<'a>(headers: &'a Headers, name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
}

fn conn_error() -> TransportError {
    TransportError::Request("connection error".into())
}

#[test]
fn get_retries_a_broken_body_and_returns_the_decoded_text() {
    let broken = FakeResponse::new(200, b"", &[])
        .chunks(vec![Ok(b"partial".to_vec()), Err("broken stream".into())]);
    let closed = broken.closed.clone();
    let good = FakeResponse::new(
        200,
        "catalogue café".as_bytes(),
        &[("Content-Type", "text/plain; charset=utf-8")],
    );
    let (client, calls, _) = make_client(global, vec![Ok(broken), Ok(good)]);
    let page = client.get("https://example.test/", None).unwrap();
    assert_eq!(page.content, "catalogue café".as_bytes());
    assert_eq!(page.text(), "catalogue café");
    assert!(closed.load(Ordering::SeqCst));
    assert_eq!(calls.lock().unwrap().len(), 2);
}

#[test]
fn get_bounds_the_stream_and_closes_an_oversized_response() {
    let chunk = vec![b'x'; http::MAX_HTML_BYTES / 2 + 1];
    let large = FakeResponse::new(200, b"", &[]).chunks(vec![
        Ok(chunk.clone()),
        Ok(chunk),
        Err("read beyond limit".into()),
    ]);
    let closed = large.closed.clone();
    let (client, calls, _) = make_client(global, vec![Ok(large)]);
    let err = client
        .get("https://example.test/", None)
        .err()
        .unwrap()
        .to_string();
    assert!(err.contains("HTML") && err.contains("limit"), "{err}");
    assert!(closed.load(Ordering::SeqCst));
    assert_eq!(calls.lock().unwrap().len(), 1);
}

#[test]
fn get_sets_headers_and_referer() {
    let (client, calls, _) = make_client(global, vec![Ok(FakeResponse::new(200, b"ok", &[]))]);
    client
        .get(
            "https://sellerx.x.yupoo.com/albums?page=1",
            Some("https://sellerx.x.yupoo.com"),
        )
        .unwrap();
    let headers = calls.lock().unwrap()[0].1.clone();
    assert!(
        header(&headers, "User-Agent")
            .unwrap()
            .contains("DiogenesLamp")
    );
    assert_eq!(
        header(&headers, "Referer"),
        Some("https://sellerx.x.yupoo.com")
    );
}

#[test]
fn get_retries_5xx_blocks_on_429_and_gives_up() {
    let (client, calls, _) = make_client(
        global,
        vec![
            Ok(FakeResponse::new(500, b"", &[])),
            Ok(FakeResponse::new(502, b"", &[])),
            Ok(FakeResponse::new(200, b"ok", &[])),
        ],
    );
    assert_eq!(client.get("https://x.example/", None).unwrap().status, 200);
    assert_eq!(calls.lock().unwrap().len(), 3);
    let (client, calls, _) = make_client(global, vec![Ok(FakeResponse::new(429, b"", &[]))]);
    assert_eq!(
        client.get("https://x.example/", None).err(),
        Some(FetchError::Blocked(429))
    );
    assert_eq!(calls.lock().unwrap().len(), 1);
    let (client, _, clock) = make_client(global, (0..RETRIES).map(|_| Err(conn_error())).collect());
    assert!(matches!(
        client.get("https://x.example/", None),
        Err(FetchError::Fetch(_))
    ));
    assert_eq!(clock.sleeps(), [1.0, 2.0]);
}

fn with_retry_after(status: u16, value: &str) -> Result<FakeResponse, TransportError> {
    Ok(FakeResponse::new(status, b"", &[("Retry-After", value)]))
}

fn ok() -> Result<FakeResponse, TransportError> {
    Ok(FakeResponse::new(200, b"ok", &[]))
}

#[test]
fn a_429_waits_once_for_retry_after_then_retries() {
    let (client, calls, clock) = make_client(global, vec![with_retry_after(429, "7"), ok()]);
    assert_eq!(client.get("https://x.example/", None).unwrap().status, 200);
    assert_eq!(calls.lock().unwrap().len(), 2);
    assert_eq!(clock.sleeps(), [7.0]);
}

#[test]
fn a_repeated_429_is_blocked_after_one_wait() {
    let (client, calls, clock) = make_client(
        global,
        vec![with_retry_after(429, "5"), with_retry_after(429, "5")],
    );
    assert_eq!(
        client.get("https://x.example/", None).err(),
        Some(FetchError::Blocked(429))
    );
    assert_eq!(calls.lock().unwrap().len(), 2);
    assert_eq!(clock.sleeps(), [5.0]);
}

#[test]
fn a_hostile_retry_after_is_capped() {
    for value in ["999999999", "99999999999999999999999999999"] {
        let (client, _, clock) = make_client(global, vec![with_retry_after(429, value), ok()]);
        assert_eq!(client.get("https://x.example/", None).unwrap().status, 200);
        assert_eq!(clock.sleeps(), [MAX_RETRY_AFTER_SECS]);
    }
    let (client, calls, clock) = make_client(
        global,
        (0..RETRIES)
            .map(|_| with_retry_after(503, "999999999"))
            .collect(),
    );
    assert!(matches!(
        client.get("https://x.example/", None),
        Err(FetchError::Fetch(_))
    ));
    assert_eq!(calls.lock().unwrap().len(), RETRIES as usize);
    assert_eq!(clock.sleeps(), [MAX_RETRY_AFTER_SECS, MAX_RETRY_AFTER_SECS]);
}

#[test]
fn retry_after_is_final_or_ignored_where_it_does_not_apply() {
    for (status, value) in [(429, "soon"), (429, "-5"), (403, "7")] {
        let (client, calls, clock) = make_client(global, vec![with_retry_after(status, value)]);
        assert_eq!(
            client.get("https://x.example/", None).err(),
            Some(FetchError::Blocked(status))
        );
        assert_eq!(calls.lock().unwrap().len(), 1);
        assert!(clock.sleeps().is_empty());
    }
    let (client, _, clock) = make_client(global, vec![with_retry_after(500, "7"), ok()]);
    assert_eq!(client.get("https://x.example/", None).unwrap().status, 200);
    assert_eq!(clock.sleeps(), [1.0]);
}

#[test]
fn a_503_retry_after_replaces_only_a_shorter_backoff() {
    let (client, _, clock) = make_client(global, vec![with_retry_after(503, "9"), ok()]);
    assert_eq!(client.get("https://x.example/", None).unwrap().status, 200);
    assert_eq!(clock.sleeps(), [9.0]);
    let (client, _, clock) = make_client(global, vec![with_retry_after(503, "0"), ok()]);
    assert_eq!(client.get("https://x.example/", None).unwrap().status, 200);
    assert_eq!(clock.sleeps(), [1.0]);
}

#[test]
fn downloads_honour_retry_after_too() {
    let dir = common::tmp();
    let dest = dir.path().join("cover.png");
    let png = png_bytes();
    let (client, calls, clock) = make_client(
        global,
        vec![
            with_retry_after(429, "7"),
            with_retry_after(503, "9"),
            Ok(FakeResponse::new(
                200,
                &png,
                &[("Content-Type", "image/png")],
            )),
        ],
    );
    assert!(client.download("https://x.example/a", &dest, None).unwrap());
    assert_eq!(calls.lock().unwrap().len(), 3);
    assert_eq!(clock.sleeps(), [7.0, 9.0]);
}

#[test]
fn an_http_date_retry_after_waits_until_that_time() {
    let date = (chrono::Utc::now() + chrono::TimeDelta::seconds(30))
        .format("%a, %d %b %Y %H:%M:%S GMT")
        .to_string();
    let (client, _, clock) = make_client(global, vec![with_retry_after(429, &date), ok()]);
    assert_eq!(client.get("https://x.example/", None).unwrap().status, 200);
    let sleeps = clock.sleeps();
    assert_eq!(sleeps.len(), 1);
    assert!((25.0..=30.0).contains(&sleeps[0]), "{sleeps:?}");
}

#[test]
fn retry_after_values_parse_in_every_form() {
    let now = chrono::DateTime::parse_from_rfc3339("2026-10-30T00:00:00Z")
        .unwrap()
        .to_utc();
    for (value, expected) in [
        ("0", Some(0.0)),
        (" 120 ", Some(MAX_RETRY_AFTER_SECS)),
        ("45", Some(45.0)),
        ("Fri, 30 Oct 2026 00:00:30 GMT", Some(30.0)),
        ("Friday, 30-Oct-26 00:00:30 GMT", Some(30.0)),
        ("Fri Oct 30 00:00:30 2026", Some(30.0)),
        ("Fri Oct  2 00:00:30 2026", Some(0.0)),
        ("Thu, 29 Oct 2026 23:59:00 GMT", Some(0.0)),
        ("Fri, 31 Dec 9999 23:59:59 GMT", Some(MAX_RETRY_AFTER_SECS)),
        ("", None),
        ("soon", None),
        ("-5", None),
        ("1.5", None),
        ("NaN", None),
        ("inf", None),
    ] {
        assert_eq!(http::retry_after_seconds(value, now), expected, "{value:?}");
    }
}

#[test]
fn rfc850_years_roll_within_50_years_of_now() {
    let now = chrono::DateTime::parse_from_rfc3339("2026-10-30T00:00:00Z")
        .unwrap()
        .to_utc();
    for (value, expected) in [
        ("Friday, 30-Oct-26 00:00:30 GMT", 30.0),
        ("Thursday, 29-Oct-26 00:00:30 GMT", 0.0),
        ("Thursday, 30-Oct-69 00:00:00 GMT", MAX_RETRY_AFTER_SECS),
        ("Thursday, 30-Oct-70 00:00:30 GMT", MAX_RETRY_AFTER_SECS),
        ("Friday, 30-Oct-76 00:00:00 GMT", MAX_RETRY_AFTER_SECS),
        ("Friday, 30-Oct-76 00:00:01 GMT", 0.0),
        ("Sunday, 30-Oct-77 00:00:00 GMT", 0.0),
    ] {
        assert_eq!(
            http::retry_after_seconds(value, now),
            Some(expected),
            "{value:?}"
        );
    }
}

fn png_bytes() -> Vec<u8> {
    common::png(2, 2, [255, 0, 0])
}

#[test]
fn download_writes_atomically_and_reports_failures() {
    let dir = common::tmp();
    let payload = png_bytes();
    let (client, _, _) = make_client(
        global,
        vec![Ok(FakeResponse::new(
            200,
            &payload,
            &[("Content-Type", "image/png")],
        ))],
    );
    let dest = dir.path().join("thumbs/s/1.jpg");
    assert!(
        client
            .download("https://photo.yupoo.com/x/small.jpg", &dest, None)
            .unwrap()
    );
    assert_eq!(std::fs::read(&dest).unwrap(), payload);
    assert_eq!(
        std::fs::read_dir(dest.parent().unwrap()).unwrap().count(),
        1
    );
    let (client, _, _) = make_client(global, vec![Ok(FakeResponse::new(404, b"", &[]))]);
    assert!(
        !client
            .download(
                "https://photo.yupoo.com/x.jpg",
                &dir.path().join("t.jpg"),
                None
            )
            .unwrap()
    );
}

fn broken_stream() -> FakeResponse {
    FakeResponse::new(200, b"", &[("Content-Type", "image/png")])
        .chunks(vec![Ok(b"partial".to_vec()), Err("died mid-body".into())])
}

#[test]
fn download_retries_broken_streams_then_cleans_up_or_succeeds() {
    let dir = common::tmp();
    let (client, calls, _) = make_client(
        global,
        vec![
            Ok(broken_stream()),
            Ok(broken_stream()),
            Ok(broken_stream()),
        ],
    );
    let dest = dir.path().join("thumbs/1.jpg");
    assert!(
        !client
            .download("https://photo.yupoo.com/x.jpg", &dest, None)
            .unwrap()
    );
    assert!(!dest.exists());
    assert_eq!(
        std::fs::read_dir(dest.parent().unwrap()).unwrap().count(),
        0
    );
    assert_eq!(calls.lock().unwrap().len(), 3);
    let payload = png_bytes();
    let (client, _, _) = make_client(
        global,
        vec![
            Ok(broken_stream()),
            Ok(FakeResponse::new(
                200,
                &payload,
                &[("Content-Type", "image/png")],
            )),
        ],
    );
    let dest = dir.path().join("2.jpg");
    assert!(
        client
            .download("https://photo.yupoo.com/x.jpg", &dest, None)
            .unwrap()
    );
    assert_eq!(std::fs::read(&dest).unwrap(), payload);
}

#[test]
fn get_rejects_empty_or_non_global_dns_answers() {
    let sets: Vec<Vec<&str>> = vec![
        vec![],
        vec!["127.0.0.1"],
        vec!["10.0.0.8"],
        vec!["169.254.10.20"],
        vec!["::1"],
        vec!["fe80::1"],
        vec!["8.8.8.8", "127.0.0.1"],
        vec!["2001:4860:4860::8888", "fc00::1"],
        vec!["2002:7f00:0001::"],
        vec!["64:ff9b::7f00:1"],
        vec!["64:ff9b:1::808:808"],
        vec!["2001:0000:4136:e378:8000:63bf:3fff:fdd2"],
    ];
    for answers in sets {
        let (client, calls, _) = make_client(
            static_answers(&answers),
            vec![Ok(FakeResponse::new(200, b"ok", &[]))],
        );
        assert!(
            matches!(
                client.get("https://catalogue.example/albums", None),
                Err(FetchError::Fetch(_))
            ),
            "{answers:?}"
        );
        assert!(calls.lock().unwrap().is_empty());
    }
}

#[test]
fn get_rejects_unsafe_url_forms() {
    for (url, answer) in [
        ("http://catalogue.example/", "8.8.8.8"),
        ("https://user@catalogue.example/", "8.8.8.8"),
        ("https://catalogue.example:8443/", "8.8.8.8"),
        ("https://localhost/", "127.0.0.1"),
        ("https://127.0.0.1/", "127.0.0.1"),
        ("https://[::ffff:127.0.0.1]/", "::ffff:127.0.0.1"),
        ("https://[fe80::1%25ethernet]/", "fe80::1"),
    ] {
        let (client, calls, _) = make_client(
            static_answers(&[answer]),
            vec![Ok(FakeResponse::new(200, b"ok", &[]))],
        );
        assert!(client.get(url, None).is_err(), "{url}");
        assert!(calls.lock().unwrap().is_empty());
    }
    for url in ["https://[::1", "https://[abc", "https://[::1]:80]/x"] {
        assert!(normalise_url(url).is_err(), "{url}");
    }
}

#[test]
fn get_pins_the_validated_address_and_keeps_the_host() {
    let (client, calls, _) = make_client(global, vec![Ok(FakeResponse::new(200, b"ok", &[]))]);
    assert_eq!(
        client
            .get("https://catalogue.example/albums", None)
            .unwrap()
            .status,
        200
    );
    let (target, headers) = calls.lock().unwrap()[0].clone();
    assert_eq!(target.connect_ips, ["8.8.8.8"]);
    assert_eq!(target.hostname, "catalogue.example");
    assert_eq!(header(&headers, "Host"), Some("catalogue.example"));
}

#[test]
fn pinned_agents_refuse_proxies_and_redirects() {
    let config = http::agent_config();
    assert!(config.proxy().is_none());
    assert_eq!(config.max_redirects(), 0);
    assert!(config.https_only());
}

#[test]
fn agent_cache_reuses_per_host_and_address_and_evicts_the_oldest() {
    let mut cache: Lru<u32> = Lru::new(http::MAX_AGENTS_PER_THREAD);
    let key = |h: &str, ip: &str| (h.to_string(), ip.to_string(), 443u16);
    let mut made = 0;
    for _ in 0..3 {
        cache
            .get_or_insert::<()>(key("catalogue.example", "8.8.8.8"), || {
                made += 1;
                Ok(made)
            })
            .unwrap();
    }
    cache
        .get_or_insert::<()>(key("catalogue.example", "1.1.1.1"), || {
            made += 1;
            Ok(made)
        })
        .unwrap();
    assert_eq!(made, 2);
    let mut evicted = Vec::new();
    for i in 0..4 {
        let (_, gone) = cache
            .get_or_insert::<()>(key(&format!("host{i}.example"), "8.8.8.8"), || Ok(100 + i))
            .unwrap();
        evicted.extend(gone);
    }
    assert_eq!(evicted, [1, 2]);
    assert_eq!(cache.len(), 4);
    assert!(cache.remove(&key("host3.example", "8.8.8.8")).is_some());
    assert!(cache.remove(&key("host3.example", "8.8.8.8")).is_none());
}

#[test]
fn redirects_are_resolved_again_and_pinned_independently() {
    let resolver = |host: &str, _: u16| -> Result<Vec<String>, String> {
        Ok(vec![
            if host == "catalogue.example" {
                "8.8.8.8"
            } else {
                "1.1.1.1"
            }
            .into(),
        ])
    };
    let (client, calls, _) = make_client(
        resolver,
        vec![
            Ok(FakeResponse::new(
                302,
                b"",
                &[("Location", "https://images.example/thumb.png")],
            )),
            Ok(FakeResponse::new(200, b"ok", &[])),
        ],
    );
    client.get("https://catalogue.example/start", None).unwrap();
    let calls = calls.lock().unwrap();
    assert_eq!(
        calls
            .iter()
            .map(|c| c.0.connect_ips.clone())
            .collect::<Vec<_>>(),
        [vec!["8.8.8.8".to_string()], vec!["1.1.1.1".to_string()]]
    );
    assert_eq!(
        calls
            .iter()
            .map(|c| header(&c.1, "Host").unwrap().to_string())
            .collect::<Vec<_>>(),
        ["catalogue.example", "images.example"]
    );
}

#[test]
fn redirect_rebinding_downgrades_missing_location_and_hop_limit() {
    let answers = Arc::new(Mutex::new(VecDeque::from([
        vec!["8.8.8.8".to_string()],
        vec!["127.0.0.1".to_string()],
    ])));
    let resolver = move |_: &str, _: u16| Ok(answers.lock().unwrap().pop_front().unwrap());
    let (client, calls, _) = make_client(
        resolver,
        vec![
            Ok(FakeResponse::new(302, b"", &[("Location", "/again")])),
            Ok(FakeResponse::new(200, b"ok", &[])),
        ],
    );
    assert!(client.get("https://catalogue.example/start", None).is_err());
    assert_eq!(calls.lock().unwrap().len(), 1);
    let (client, calls, _) = make_client(
        global,
        vec![Ok(FakeResponse::new(
            302,
            b"",
            &[("Location", "http://catalogue.example/plain")],
        ))],
    );
    assert!(client.get("https://catalogue.example/start", None).is_err());
    assert_eq!(calls.lock().unwrap().len(), 1);
    let redirect = FakeResponse::new(302, b"", &[]);
    let closed = redirect.closed.clone();
    let (client, _, _) = make_client(global, vec![Ok(redirect)]);
    assert!(client.get("https://catalogue.example/start", None).is_err());
    assert!(closed.load(Ordering::SeqCst));
    let (client, calls, _) = make_client(
        global,
        (0..=MAX_REDIRECTS)
            .map(|_| Ok(FakeResponse::new(302, b"", &[("Location", "/again")])))
            .collect(),
    );
    assert!(client.get("https://catalogue.example/start", None).is_err());
    assert_eq!(calls.lock().unwrap().len() as u32, MAX_REDIRECTS + 1);
}

fn png_with_dimensions(width: u32, height: u32) -> Vec<u8> {
    fn chunk(kind: &[u8], data: &[u8]) -> Vec<u8> {
        let mut body = kind.to_vec();
        body.extend_from_slice(data);
        let mut out = (data.len() as u32).to_be_bytes().to_vec();
        out.extend_from_slice(&body);
        let mut crc = 0xffff_ffffu32;
        for &b in &body {
            crc ^= u32::from(b);
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0xedb8_8320
                } else {
                    crc >> 1
                };
            }
        }
        out.extend_from_slice(&(!crc).to_be_bytes());
        out
    }
    let mut header = width.to_be_bytes().to_vec();
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[8, 2, 0, 0, 0]);
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    out.extend(chunk(b"IHDR", &header));
    out.extend(chunk(
        b"IDAT",
        &[
            0x78, 0x9c, 0x63, 0x60, 0x60, 0x60, 0x60, 0x00, 0x00, 0x00, 0x05, 0x00, 0x01,
        ],
    ));
    out.extend(chunk(b"IEND", b""));
    out
}

fn download_into(dir: &std::path::Path, response: FakeResponse, name: &str) -> bool {
    let (client, _, _) = make_client(global, vec![Ok(response)]);
    client
        .download("https://images.example/thumb.png", &dir.join(name), None)
        .unwrap()
}

#[test]
fn download_bounds_mime_dimensions_and_truncation() {
    let dir = common::tmp();
    let payload = png_bytes();
    let length = payload.len().to_string();
    assert!(download_into(
        dir.path(),
        FakeResponse::new(
            200,
            &payload,
            &[("Content-Type", "image/png"), ("Content-Length", &length)]
        ),
        "thumb.png"
    ));
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    assert!(download_into(
        dir.path(),
        FakeResponse::new(
            200,
            &payload,
            &[("Content-Type", "Image/PNG; charset=binary")]
        ),
        "b.png"
    ));

    let dir = common::tmp();
    let dest = dir.path().join("thumb.png");
    common::write(&dest, b"old-image");
    let too_long = (MAX_IMAGE_BYTES + 1).to_string();
    assert!(!download_into(
        dir.path(),
        FakeResponse::new(
            200,
            &payload,
            &[("Content-Type", "image/png"), ("Content-Length", &too_long)]
        ),
        "thumb.png"
    ));
    assert_eq!(std::fs::read(&dest).unwrap(), b"old-image");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);

    for response in [
        FakeResponse::new(200, b"", &[("Content-Type", "image/png")]).chunks(vec![
            Ok(vec![b'x'; (MAX_IMAGE_BYTES / 2 + 1) as usize]),
            Ok(vec![b'y'; (MAX_IMAGE_BYTES / 2 + 1) as usize]),
        ]),
        FakeResponse::new(200, &payload, &[("Content-Type", "image/jpeg")]),
        FakeResponse::new(
            200,
            &png_with_dimensions(8193, 1),
            &[("Content-Type", "image/png")],
        ),
        FakeResponse::new(
            200,
            &png_with_dimensions(4473, 4473),
            &[("Content-Type", "image/png")],
        ),
        FakeResponse::new(
            200,
            &payload[..payload.len() - 12],
            &[("Content-Type", "image/png")],
        ),
    ] {
        let dir = common::tmp();
        assert!(!download_into(dir.path(), response, "thumb.png"));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}

fn target() -> Target {
    Target {
        url: "https://example/".into(),
        hostname: "example".into(),
        connect_ips: vec!["8.8.8.8".into(), "1.1.1.1".into()],
        port: 443,
    }
}

#[test]
fn every_validated_address_is_tried_and_proxy_errors_stop_at_once() {
    let tried = Mutex::new(Vec::new());
    let result = open_each_address(&target(), &Vec::new(), |_, ip, _| {
        tried.lock().unwrap().push(ip.to_string());
        if ip == "1.1.1.1" {
            Ok(Box::new(FakeResponse::new(200, b"", &[])) as Box<dyn Response>)
        } else {
            Err(TransportError::Request("refused".into()))
        }
    });
    assert_eq!(result.unwrap().status(), 200);
    assert_eq!(*tried.lock().unwrap(), ["8.8.8.8", "1.1.1.1"]);
    let last = open_each_address(&target(), &Vec::new(), |_, ip, _| {
        Err(TransportError::Request(format!("refused {ip}")))
    });
    assert!(matches!(last, Err(TransportError::Request(m)) if m.contains("1.1.1.1")));
    let tried = Mutex::new(Vec::new());
    let proxy = open_each_address(&target(), &Vec::new(), |_, ip, _| {
        tried.lock().unwrap().push(ip.to_string());
        Err(TransportError::Proxy(
            "safe fetch does not permit proxy routing".into(),
        ))
    });
    assert!(matches!(proxy, Err(TransportError::Proxy(_))));
    assert_eq!(*tried.lock().unwrap(), ["8.8.8.8"]);
}
