//! Polite, rebinding-safe HTTP client with bounded atomic image downloads.

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Read;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::covers;
use crate::ipaddr::{self, Ip};
use crate::pyerr;
use crate::pystr;
use crate::pyurl;

pub const USER_AGENT: &str = "DiogenesLamp/0.1 (+https://github.com/ryanduguid/DiogenesLamp)";
pub const RETRIES: u32 = 3;
pub const MAX_REDIRECTS: u32 = 5;
pub const MAX_IMAGE_BYTES: u64 = 8 * 1024 * 1024;
pub const MAX_HTML_BYTES: usize = 8 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(30);
const IMAGE_MIMES: [(&str, &[&str]); 3] = [
    ("JPEG", &["image/jpeg", "image/jpg"]),
    ("PNG", &["image/png"]),
    ("WEBP", &["image/webp"]),
];

/// FetchError, or its BlockedError subclass.
#[derive(Clone, Debug, PartialEq)]
pub enum FetchError {
    Fetch(String),
    Blocked(u16),
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchError::Fetch(m) => f.write_str(m),
            FetchError::Blocked(status) => write!(f, "blocked with HTTP {status}"),
        }
    }
}

enum Failure {
    Final(FetchError),
    Retryable(String),
}

/// The clock and sleep a limiter and client use; tests substitute them.
pub trait Clock: Send + Sync {
    fn now(&self) -> f64;
    fn sleep(&self, seconds: f64);
}

pub struct SystemClock {
    start: Instant,
}

impl Default for SystemClock {
    fn default() -> Self {
        SystemClock {
            start: Instant::now(),
        }
    }
}

impl Clock for SystemClock {
    fn now(&self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }

    fn sleep(&self, seconds: f64) {
        // Differential builds answer from local fixtures and never wait.
        if seconds > 0.0 && !cfg!(feature = "fixture-transport") {
            std::thread::sleep(Duration::from_secs_f64(seconds));
        }
    }
}

pub struct RateLimiter {
    global_gap: f64,
    host_gap: f64,
    overrides: HashMap<String, f64>,
    clock: Arc<dyn Clock>,
    state: Mutex<(f64, HashMap<String, f64>)>,
}

impl RateLimiter {
    pub fn new(
        global_rps: f64,
        host_rps: f64,
        overrides: &[(&str, f64)],
        clock: Arc<dyn Clock>,
    ) -> RateLimiter {
        RateLimiter {
            global_gap: 1.0 / global_rps,
            host_gap: 1.0 / host_rps,
            overrides: overrides
                .iter()
                .map(|(h, rps)| ((*h).to_string(), 1.0 / rps))
                .collect(),
            clock,
            state: Mutex::new((f64::NEG_INFINITY, HashMap::new())),
        }
    }

    /// Admit against the clock after every sleep: other hosts can proceed
    /// while this host waits, so a reserved global slot may no longer fit.
    pub fn wait(&self, host: &str) {
        if cfg!(feature = "fixture-transport") {
            return;
        }
        loop {
            let delay = {
                let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
                let now = self.clock.now();
                let host_gap = self.overrides.get(host).copied().unwrap_or(self.host_gap);
                let last_host = state.1.get(host).copied().unwrap_or(f64::NEG_INFINITY);
                let ready = (state.0 + self.global_gap).max(last_host + host_gap);
                let delay = ready - now;
                if delay <= 0.0 {
                    state.0 = now;
                    state.1.insert(host.to_string(), now);
                    return;
                }
                delay
            };
            self.clock.sleep(delay);
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub url: String,
    pub hostname: String,
    /// Every validated address, in resolution order.
    pub connect_ips: Vec<String>,
    pub port: u16,
}

pub type Headers = Vec<(String, String)>;

/// A response whose body is read in decoded chunks.
pub trait Response: Send {
    fn status(&self) -> u16;
    fn header(&self, name: &str) -> Option<String>;
    /// Ok(None) at the end; Err is a broken stream (RequestException).
    fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, String>;
}

#[derive(Debug)]
pub enum TransportError {
    /// ProxyError: refused by design, never retried on another address.
    Proxy(String),
    /// RequestException or OSError.
    Request(String),
}

pub type Resolver = dyn Fn(&str, u16) -> Result<Vec<String>, String> + Send + Sync;
pub type Transport =
    dyn Fn(&Target, &Headers) -> Result<Box<dyn Response>, TransportError> + Send + Sync;

pub fn host_header(hostname: &str) -> String {
    match ipaddr::ip_address(hostname) {
        Ok((Ip::V6(_), _)) => format!("[{hostname}]"),
        _ => hostname.to_string(),
    }
}

/// Python's built-in "idna" codec on a host (IDNA 2003 for non-ASCII).
fn idna_encode(host: &str) -> Option<String> {
    if host.is_ascii() {
        let labels: Vec<&str> = host.split('.').collect();
        if labels[..labels.len() - 1].iter().any(|l| l.is_empty())
            || labels.iter().any(|l| l.len() >= 64)
        {
            return None;
        }
        return Some(host.to_string());
    }
    // shortcut: UTS 46 processing stands in for IDNA 2003 nameprep here; the
    // two differ for a few characters (such as ß), which only matters for a
    // non-ASCII host in a fetched URL.
    idna::domain_to_ascii(host).ok()
}

/// `_normalise_url`: (normalised URL, hostname, port).
pub fn normalise_url(url: &str) -> Result<(String, String, u16), String> {
    if url.is_empty() {
        return Err("refusing empty URL".into());
    }
    if url.contains('\\') || url.chars().any(|c| (c as u32) < 32 || c as u32 == 127) {
        return Err(format!("refusing malformed URL: {url}"));
    }
    let parts = pyurl::urlsplit(url).map_err(|_| format!("refusing malformed URL: {url}"))?;
    if parts.scheme.to_lowercase() != "https" {
        return Err(format!("refusing non-https URL: {url}"));
    }
    if parts.username().is_some() || parts.password().is_some() {
        return Err(format!("refusing URL with userinfo: {url}"));
    }
    let port = parts
        .port()
        .map_err(|_| format!("refusing invalid port in URL: {url}"))?;
    let port = match port {
        None | Some(0) => 443,
        Some(p) => p,
    };
    if port != 443 {
        return Err(format!("refusing non-default HTTPS port: {url}"));
    }
    let raw = parts.hostname().unwrap_or_default();
    if raw.is_empty() || raw.contains('%') {
        return Err(format!("refusing invalid host: {url}"));
    }
    let hostname = match ipaddr::ip_address(&raw) {
        Ok((ip, scope)) => ipaddr::ip_str(ip, scope.as_deref()),
        Err(_) => idna_encode(raw.trim_end_matches('.'))
            .ok_or_else(|| format!("refusing invalid host: {url}"))?
            .to_lowercase(),
    };
    if hostname.is_empty() || hostname == "localhost" || hostname.ends_with(".local") {
        return Err(format!("refusing local host: {url}"));
    }
    let path = if parts.path.is_empty() {
        "/"
    } else {
        parts.path.as_str()
    };
    let normalised = pyurl::urlunsplit(
        "https",
        &host_header(&hostname),
        path,
        parts.query_str(),
        "",
    );
    Ok((normalised, hostname, port as u16))
}

/// System DNS, as `socket.getaddrinfo` sees it.
pub fn default_resolver(hostname: &str, port: u16) -> Result<Vec<String>, String> {
    use std::net::ToSocketAddrs;
    let addrs = (hostname, port)
        .to_socket_addrs()
        .map_err(|e| match e.raw_os_error() {
            Some(code) if cfg!(windows) => format!("[Errno {code}] getaddrinfo failed"),
            _ => e.to_string(),
        })?;
    Ok(addrs.map(|a| a.ip().to_string()).collect())
}

fn is_global_route(ip: Ip) -> bool {
    match ip {
        Ip::V4(v) => ipaddr::v4_is_global(v),
        Ip::V6(v) => {
            if !ipaddr::v6_is_global(v) {
                return false;
            }
            if let Some(mapped) = ipaddr::ipv4_mapped(v) {
                return ipaddr::v4_is_global(mapped);
            }
            if let Some(embedded) = ipaddr::sixtofour(v) {
                return ipaddr::v4_is_global(embedded);
            }
            if ipaddr::is_teredo(v) || (v >> 80) == 0x0064_ff9b_0001 {
                return false;
            }
            if (v >> 32) == 0x0064_ff9b_0000_0000_0000_0000 {
                return ipaddr::v4_is_global(v as u32);
            }
            true
        }
    }
}

pub fn validated_target(url: &str, resolver: &Resolver) -> Result<Target, String> {
    let (normalised, hostname, port) = normalise_url(url)?;
    let answers = resolver(&hostname, port)
        .map_err(|e| format!("DNS resolution failed for {hostname}: {e}"))?;
    if answers.is_empty() {
        return Err(format!("DNS returned no addresses for {hostname}"));
    }
    let mut validated: Vec<String> = Vec::new();
    for text in answers {
        if text.contains('%') {
            return Err(format!(
                "DNS returned scoped address for {hostname}: {text}"
            ));
        }
        let (ip, _) = ipaddr::ip_address(&text)
            .map_err(|_| format!("DNS returned invalid address for {hostname}: {text}"))?;
        if !is_global_route(ip) {
            return Err(format!(
                "DNS returned non-global address for {hostname}: {text}"
            ));
        }
        let canonical = ipaddr::ip_str(ip, None);
        if !validated.contains(&canonical) {
            validated.push(canonical);
        }
    }
    Ok(Target {
        url: normalised,
        hostname,
        connect_ips: validated,
        port,
    })
}

fn content_type(response: &dyn Response) -> String {
    response
        .header("Content-Type")
        .unwrap_or_default()
        .split(';')
        .next()
        .map(|s| pystr::strip(s).to_lowercase())
        .unwrap_or_default()
}

/// A buffered page: `response.status_code`, `.content` and `.text`.
pub struct Page {
    pub status: u16,
    pub content: Vec<u8>,
    pub content_type: Option<String>,
}

impl Page {
    /// requests' `Response.text`: the declared charset, ISO-8859-1 for text/*
    /// without one, otherwise UTF-8 (standing in for charset detection).
    pub fn text(&self) -> String {
        let declared = self.content_type.as_deref().and_then(|ct| {
            let mut parts = ct.split(';');
            let main = parts.next().unwrap_or("").trim().to_lowercase();
            for param in parts {
                if let Some((k, v)) = param.split_once('=')
                    && k.trim().eq_ignore_ascii_case("charset")
                {
                    return Some(v.trim().trim_matches(['\'', '"']).to_string());
                }
            }
            if main.contains("text") {
                Some("ISO-8859-1".into())
            } else if main.contains("application/json") {
                Some("utf-8".into())
            } else {
                None
            }
        });
        let codec = declared
            .map(|c| c.to_lowercase().replace('_', "-"))
            .unwrap_or_else(|| "utf-8".into());
        match codec.as_str() {
            "iso-8859-1" | "iso8859-1" | "latin-1" | "latin1" | "l1" | "8859" | "iso8859"
            | "cp819" | "iso-ir-100" => self.content.iter().map(|&b| b as char).collect(),
            "ascii" | "us-ascii" | "646" => self
                .content
                .iter()
                .map(|&b| if b < 128 { b as char } else { '\u{fffd}' })
                .collect(),
            _ => String::from_utf8_lossy(&self.content).into_owned(),
        }
    }
}

pub struct Client {
    limiter: Arc<RateLimiter>,
    resolver: Arc<Resolver>,
    transport: Arc<Transport>,
    user_agent: String,
    clock: Arc<dyn Clock>,
}

impl Client {
    pub fn new(limiter: Arc<RateLimiter>) -> Client {
        Client::with_parts(
            limiter,
            default_resolver_for_build(),
            default_transport(),
            Arc::new(SystemClock::default()),
        )
    }

    pub fn with_parts(
        limiter: Arc<RateLimiter>,
        resolver: Arc<Resolver>,
        transport: Arc<Transport>,
        clock: Arc<dyn Clock>,
    ) -> Client {
        Client {
            limiter,
            resolver,
            transport,
            user_agent: USER_AGENT.into(),
            clock,
        }
    }

    fn follow_redirects(
        &self,
        url: &str,
        referer: Option<&str>,
    ) -> Result<Box<dyn Response>, Failure> {
        let mut current = url.to_string();
        for hop in 0..=MAX_REDIRECTS {
            let target = validated_target(&current, &*self.resolver)
                .map_err(|e| Failure::Final(FetchError::Fetch(e)))?;
            let mut headers: Headers = vec![
                ("User-Agent".into(), self.user_agent.clone()),
                ("Host".into(), host_header(&target.hostname)),
            ];
            if let Some(referer) = referer.filter(|r| !r.is_empty()) {
                headers.push(("Referer".into(), referer.to_string()));
            }
            self.limiter.wait(&target.hostname);
            let response = (self.transport)(&target, &headers).map_err(|e| {
                let text = match e {
                    TransportError::Proxy(m) | TransportError::Request(m) => m,
                };
                Failure::Retryable(format!("request failed for {}: {text}", target.url))
            })?;
            let status = response.status();
            if matches!(status, 301 | 302 | 303 | 307 | 308) {
                let location =
                    pystr::strip(&response.header("Location").unwrap_or_default()).to_string();
                drop(response);
                if location.is_empty() {
                    return Err(Failure::Final(FetchError::Fetch(format!(
                        "redirect missing Location for {}",
                        target.url
                    ))));
                }
                if hop >= MAX_REDIRECTS {
                    return Err(Failure::Final(FetchError::Fetch(format!(
                        "too many redirects for {url}"
                    ))));
                }
                current = pyurl::urljoin(&target.url, &location).map_err(|_| {
                    Failure::Final(FetchError::Fetch(format!(
                        "refusing malformed URL: {location}"
                    )))
                })?;
                continue;
            }
            if status == 429 || status == 403 {
                return Err(Failure::Final(FetchError::Blocked(status)));
            }
            if (500..600).contains(&status) {
                return Err(Failure::Retryable(format!(
                    "HTTP {status} for {}",
                    target.url
                )));
            }
            if status != 200 {
                return Err(Failure::Final(FetchError::Fetch(format!(
                    "HTTP {status} for {}",
                    target.url
                ))));
            }
            return Ok(response);
        }
        Err(Failure::Final(FetchError::Fetch(
            "redirect loop exhausted without returning".into(),
        )))
    }

    fn fetch_response(
        &self,
        url: &str,
        referer: Option<&str>,
    ) -> Result<Box<dyn Response>, FetchError> {
        let mut last_error = String::new();
        for attempt in 0..RETRIES {
            match self.follow_redirects(url, referer) {
                Ok(response) => return Ok(response),
                Err(Failure::Final(error)) => return Err(error),
                Err(Failure::Retryable(message)) => {
                    last_error = message;
                    if attempt + 1 < RETRIES {
                        self.clock.sleep(2f64.powi(attempt as i32));
                    }
                }
            }
        }
        Err(FetchError::Fetch(format!("gave up on {url}: {last_error}")))
    }

    /// A buffered GET, bounded to 8 MiB.
    pub fn get(&self, url: &str, referer: Option<&str>) -> Result<Page, FetchError> {
        let mut last_error = String::new();
        for attempt in 0..RETRIES {
            let failure = match self.follow_redirects(url, referer) {
                Err(Failure::Final(error)) => return Err(error),
                Err(Failure::Retryable(message)) => message,
                Ok(mut response) => {
                    let mut body = Vec::new();
                    let read = loop {
                        match response.next_chunk() {
                            Ok(None) => break Ok(()),
                            Ok(Some(chunk)) => {
                                if body.len() + chunk.len() > MAX_HTML_BYTES {
                                    return Err(FetchError::Fetch(format!(
                                        "HTML body exceeds {MAX_HTML_BYTES}-byte limit"
                                    )));
                                }
                                body.extend_from_slice(&chunk);
                            }
                            Err(e) => break Err(e),
                        }
                    };
                    match read {
                        Ok(()) => {
                            return Ok(Page {
                                status: response.status(),
                                content_type: response.header("Content-Type"),
                                content: body,
                            });
                        }
                        Err(e) => format!("body read failed for {url}: {e}"),
                    }
                }
            };
            last_error = failure;
            if attempt + 1 < RETRIES {
                self.clock.sleep(2f64.powi(attempt as i32));
            }
        }
        Err(FetchError::Fetch(format!("gave up on {url}: {last_error}")))
    }

    /// Ok(None) is a deterministic refusal (False); Err(stream) is retried.
    fn write_bounded_image(
        &self,
        response: &mut dyn Response,
        dest: &Path,
    ) -> Result<bool, String> {
        let mime = content_type(response);
        if !IMAGE_MIMES
            .iter()
            .any(|(_, mimes)| mimes.contains(&mime.as_str()))
        {
            return Ok(false);
        }
        if let Some(declared) = response.header("Content-Length") {
            match pystr::parse_int(&declared) {
                Some(n) if (0..=i128::from(MAX_IMAGE_BYTES)).contains(&n) => {}
                _ => return Ok(false),
            }
        }
        let parent = dest.parent().unwrap_or(Path::new("."));
        if pyerr::mkdir_parents(parent).is_err() {
            return Ok(false);
        }
        let name = dest
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let Ok((tmp, mut file)) = crate::storage::create_temp(parent, &format!("{name}."), ".tmp")
        else {
            return Ok(false);
        };
        let outcome = (|| -> Result<bool, String> {
            use std::io::Write;
            let mut observed: u64 = 0;
            loop {
                match response.next_chunk() {
                    Ok(None) => break,
                    Ok(Some(chunk)) => {
                        if chunk.is_empty() {
                            continue;
                        }
                        observed += chunk.len() as u64;
                        if observed > MAX_IMAGE_BYTES {
                            return Ok(false);
                        }
                        if file.write_all(&chunk).is_err() {
                            return Ok(false);
                        }
                    }
                    Err(e) => return Err(e),
                }
            }
            drop(file);
            if observed == 0 {
                return Ok(false);
            }
            if verify_image(&tmp, &mime).is_err() || covers::resize_cover(&tmp).is_err() {
                return Ok(false);
            }
            Ok(std::fs::rename(&tmp, dest).is_ok())
        })();
        if outcome != Ok(true) {
            let _ = std::fs::remove_file(&tmp);
        }
        outcome
    }

    /// Fetch a bounded image into dest atomically. Ok(false) means this
    /// attempt failed, not that dest is absent.
    pub fn download(
        &self,
        url: &str,
        dest: &Path,
        referer: Option<&str>,
    ) -> Result<bool, FetchError> {
        for attempt in 0..RETRIES {
            let mut response = match self.fetch_response(url, referer) {
                Ok(response) => response,
                Err(FetchError::Blocked(status)) => return Err(FetchError::Blocked(status)),
                Err(FetchError::Fetch(_)) => return Ok(false),
            };
            match self.write_bounded_image(&mut *response, dest) {
                Ok(result) => return Ok(result),
                Err(_) => {
                    if attempt + 1 < RETRIES {
                        self.clock.sleep(2f64.powi(attempt as i32));
                    }
                }
            }
        }
        Ok(false)
    }
}

/// `_verify_image`: decodable within bounds, and the MIME matches.
pub fn verify_image(path: &Path, mime: &str) -> Result<(), String> {
    let opened = covers::open(path).map_err(|e| e.to_string())?;
    if opened.width == 0 || opened.height == 0 {
        return Err("image has invalid dimensions".into());
    }
    if opened.width > 8192 || opened.height > 8192 {
        return Err("image dimensions exceed limit".into());
    }
    if u64::from(opened.width) * u64::from(opened.height) > 20_000_000 {
        return Err("image pixel count exceeds limit".into());
    }
    let allowed = IMAGE_MIMES
        .iter()
        .find(|(format, _)| *format == opened.format_name())
        .is_some_and(|(_, mimes)| mimes.contains(&mime));
    if !allowed {
        return Err("image MIME does not match decoded format".into());
    }
    opened
        .decode()
        .map(|_| ())
        .map_err(|e| format!("invalid image: {e}"))
}

// --- The real transport: pinned connections over rustls. ---

#[derive(Debug)]
struct PinnedResolver {
    addr: std::net::SocketAddr,
}

impl ureq::unversioned::resolver::Resolver for PinnedResolver {
    fn resolve(
        &self,
        _uri: &ureq::http::Uri,
        _config: &ureq::config::Config,
        _timeout: ureq::unversioned::transport::NextTimeout,
    ) -> Result<ureq::unversioned::resolver::ResolvedSocketAddrs, ureq::Error> {
        let mut addrs = self.empty();
        addrs.push(self.addr);
        Ok(addrs)
    }
}

/// Most-recently-used cache: entries per key, the oldest evicted past `max`.
pub struct Lru<V> {
    max: usize,
    entries: Vec<((String, String, u16), V)>,
}

impl<V: Clone> Lru<V> {
    pub const fn new(max: usize) -> Lru<V> {
        Lru {
            max,
            entries: Vec::new(),
        }
    }

    /// The cached value for `key`, or a new one from `make`; Err from
    /// `make` is returned and nothing is cached.
    pub fn get_or_insert<E>(
        &mut self,
        key: (String, String, u16),
        make: impl FnOnce() -> Result<V, E>,
    ) -> Result<(V, Vec<V>), E> {
        if let Some(index) = self.entries.iter().position(|(k, _)| *k == key) {
            let entry = self.entries.remove(index);
            let value = entry.1.clone();
            self.entries.push(entry);
            return Ok((value, Vec::new()));
        }
        let value = make()?;
        self.entries.push((key, value.clone()));
        let mut evicted = Vec::new();
        while self.entries.len() > self.max {
            evicted.push(self.entries.remove(0).1);
        }
        Ok((value, evicted))
    }

    pub fn remove(&mut self, key: &(String, String, u16)) -> Option<V> {
        let index = self.entries.iter().position(|(k, _)| k == key)?;
        Some(self.entries.remove(index).1)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

thread_local! {
    /// Agents per (hostname, address, port), so a worker reuses keep-alive
    /// connections. The key includes the validated address: a host that
    /// resolves elsewhere gets a new agent pinned there.
    static AGENTS: RefCell<Lru<ureq::Agent>> = const { RefCell::new(Lru::new(MAX_AGENTS_PER_THREAD)) };
}
pub const MAX_AGENTS_PER_THREAD: usize = 4;

/// The configuration every pinned agent uses: no proxy, no automatic
/// redirects, no cookies (the cookies feature is off), HTTPS only.
pub fn agent_config() -> ureq::config::Config {
    ureq::Agent::config_builder()
        .proxy(None)
        .max_redirects(0)
        .http_status_as_error(false)
        .https_only(true)
        .user_agent(ureq::config::AutoHeaderValue::None)
        .accept(ureq::config::AutoHeaderValue::None)
        .accept_encoding(ureq::config::AutoHeaderValue::None)
        .timeout_connect(Some(TIMEOUT))
        .timeout_recv_response(Some(TIMEOUT))
        .timeout_recv_body(Some(Duration::from_secs(120)))
        .build()
}

fn pinned_agent(target: &Target, ip: &str) -> Result<ureq::Agent, TransportError> {
    let key = (target.hostname.clone(), ip.to_string(), target.port);
    let addr: std::net::IpAddr = ip
        .parse()
        .map_err(|_| TransportError::Request(format!("bad address {ip}")))?;
    let port = target.port;
    AGENTS.with(|cache| {
        cache
            .borrow_mut()
            .get_or_insert(key, || {
                Ok(ureq::Agent::with_parts(
                    agent_config(),
                    ureq::unversioned::transport::DefaultConnector::default(),
                    PinnedResolver {
                        addr: std::net::SocketAddr::new(addr, port),
                    },
                ))
            })
            .map(|(agent, _evicted)| agent)
    })
}

fn drop_agent(target: &Target, ip: &str) {
    let key = (target.hostname.clone(), ip.to_string(), target.port);
    AGENTS.with(|cache| cache.borrow_mut().remove(&key));
}

struct UreqResponse {
    status: u16,
    headers: Vec<(String, String)>,
    reader: Box<dyn Read + Send>,
}

impl Response for UreqResponse {
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
        let mut buffer = vec![0u8; 65536];
        match self.reader.read(&mut buffer) {
            Ok(0) => Ok(None),
            Ok(n) => {
                buffer.truncate(n);
                Ok(Some(buffer))
            }
            Err(e) => Err(e.to_string()),
        }
    }
}

fn open_pinned_to(
    target: &Target,
    ip: &str,
    headers: &Headers,
) -> Result<Box<dyn Response>, TransportError> {
    let agent = pinned_agent(target, ip)?;
    let mut request = agent.get(&target.url);
    for (name, value) in headers {
        if name.eq_ignore_ascii_case("Host") {
            continue;
        }
        request = request.header(name.as_str(), value.as_str());
    }
    request = request
        .header("Accept-Encoding", "gzip, deflate")
        .header("Accept", "*/*")
        .header("Connection", "keep-alive");
    let response = match request.call() {
        Ok(response) => response,
        Err(e) => {
            drop_agent(target, ip);
            return Err(TransportError::Request(e.to_string()));
        }
    };
    let status = response.status().as_u16();
    let headers: Vec<(String, String)> = response
        .headers()
        .iter()
        .map(|(k, v)| {
            (
                k.as_str().to_string(),
                v.as_bytes().iter().map(|&b| b as char).collect(),
            )
        })
        .collect();
    let encoding = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("Content-Encoding"))
        .map(|(_, v)| v.trim().to_lowercase())
        .unwrap_or_default();
    let raw: Box<dyn Read + Send> = Box::new(
        response
            .into_body()
            .into_with_config()
            .limit(u64::MAX)
            .reader(),
    );
    let reader: Box<dyn Read + Send> = match encoding.as_str() {
        "gzip" | "x-gzip" => Box::new(flate2::read::MultiGzDecoder::new(raw)),
        // shortcut: urllib3 also accepts a raw deflate stream here; zlib is
        // what servers send in practice.
        "deflate" => Box::new(flate2::read::ZlibDecoder::new(raw)),
        _ => raw,
    };
    Ok(Box::new(UreqResponse {
        status,
        headers,
        reader,
    }))
}

/// Try each validated address; a proxy refusal stops at once.
pub fn open_pinned(
    target: &Target,
    headers: &Headers,
) -> Result<Box<dyn Response>, TransportError> {
    open_each_address(target, headers, open_pinned_to)
}

/// One attempt per validated address, in resolution order.
pub fn open_each_address(
    target: &Target,
    headers: &Headers,
    open: impl Fn(&Target, &str, &Headers) -> Result<Box<dyn Response>, TransportError>,
) -> Result<Box<dyn Response>, TransportError> {
    let mut last = None;
    for ip in &target.connect_ips {
        match open(target, ip, headers) {
            Ok(response) => return Ok(response),
            Err(TransportError::Proxy(m)) => return Err(TransportError::Proxy(m)),
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| TransportError::Request("no validated address".into())))
}

#[cfg(not(feature = "fixture-transport"))]
pub fn default_transport() -> Arc<Transport> {
    Arc::new(open_pinned)
}

#[cfg(feature = "fixture-transport")]
pub fn default_transport() -> Arc<Transport> {
    Arc::new(fixture::open)
}

/// Differential harness only: answers from `$LAMP_FIXTURE_DIR/responses.json`,
/// `{url: {"status": 200, "headers": {...}, "body": "file"}}`.
#[cfg(feature = "fixture-transport")]
pub mod fixture {
    use super::*;

    struct Fixed {
        status: u16,
        headers: Vec<(String, String)>,
        body: Option<Vec<u8>>,
    }

    impl Response for Fixed {
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
            Ok(self.body.take().filter(|b| !b.is_empty()))
        }
    }

    pub fn resolver(_host: &str, _port: u16) -> Result<Vec<String>, String> {
        Ok(vec!["93.184.216.34".into()])
    }

    pub fn open(target: &Target, _headers: &Headers) -> Result<Box<dyn Response>, TransportError> {
        let dir = std::env::var("LAMP_FIXTURE_DIR").unwrap_or_default();
        let dir = std::path::PathBuf::from(dir);
        let manifest = std::fs::read_to_string(dir.join("responses.json")).unwrap_or_default();
        let manifest = crate::pyjson::loads(&manifest).unwrap_or(crate::pyjson::Value::Null);
        let entry = manifest
            .as_object()
            .and_then(|m| m.get(&target.url))
            .and_then(|e| e.as_object());
        let Some(entry) = entry else {
            return Ok(Box::new(Fixed {
                status: 404,
                headers: Vec::new(),
                body: Some(Vec::new()),
            }));
        };
        let status = match entry.get("status") {
            Some(crate::pyjson::Value::Int(n)) => *n as u16,
            _ => 200,
        };
        let headers = entry
            .get("headers")
            .and_then(|h| h.as_object())
            .map(|h| h.iter().map(|(k, v)| (k.clone(), v.py_str())).collect())
            .unwrap_or_default();
        let body = entry
            .get("body")
            .and_then(|b| b.as_str())
            .map(|name| std::fs::read(dir.join(name)).unwrap_or_default())
            .unwrap_or_default();
        Ok(Box::new(Fixed {
            status,
            headers,
            body: Some(body),
        }))
    }
}

#[cfg(feature = "fixture-transport")]
pub fn default_resolver_for_build() -> Arc<Resolver> {
    Arc::new(fixture::resolver)
}

#[cfg(not(feature = "fixture-transport"))]
pub fn default_resolver_for_build() -> Arc<Resolver> {
    Arc::new(default_resolver)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsafe_urls_are_refused() {
        for url in [
            "http://catalogue.example/",
            "https://user@catalogue.example/",
            "https://catalogue.example:8443/",
            "https://localhost/",
            "https://[::1",
            "https://[abc",
            "https://[::1]:80]/x",
            "https://a..b/",
        ] {
            assert!(normalise_url(url).is_err(), "{url}");
        }
        assert_eq!(
            normalise_url("https://Catalogue.Example").unwrap(),
            (
                "https://catalogue.example/".to_string(),
                "catalogue.example".to_string(),
                443
            )
        );
    }
}
