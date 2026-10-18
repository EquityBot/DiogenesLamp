//! Ported from tests/test_serve.py and tests/wheel_smoke.py.
mod common;

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use common::write;
use lamp::serve::{REFUSED, Server, Viewer, make_server};

struct Running {
    port: u16,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Running {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn start(viewer: Viewer, data: &Path) -> Running {
    let server: Server = make_server(viewer, data, 0).unwrap();
    let port = server.port();
    let stop = server.shutdown_handle();
    let thread = std::thread::spawn(move || server.serve_forever());
    Running {
        port,
        stop,
        thread: Some(thread),
    }
}

struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Reply {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

fn request(port: u16, method: &str, path: &str, headers: &[(&str, &str)]) -> Reply {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let mut raw = format!("{method} {path} HTTP/1.0\r\n");
    for (k, v) in headers {
        raw.push_str(&format!("{k}: {v}\r\n"));
    }
    raw.push_str("\r\n");
    stream.write_all(raw.as_bytes()).unwrap();
    let mut data = Vec::new();
    stream.read_to_end(&mut data).unwrap();
    let split = data
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .unwrap_or(data.len());
    let head = String::from_utf8_lossy(&data[..split]).into_owned();
    let body = data.get(split + 4..).unwrap_or_default().to_vec();
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .unwrap_or("")
        .split(' ')
        .nth(1)
        .unwrap_or("0")
        .parse()
        .unwrap_or(0);
    let headers = lines
        .filter_map(|l| l.split_once(": "))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    Reply {
        status,
        headers,
        body,
    }
}

fn get(port: u16, path: &str) -> Reply {
    request(port, "GET", path, &[])
}

fn layout(dir: &Path) -> (PathBuf, PathBuf) {
    let viewer = dir.join("viewer");
    let data = dir.join("data");
    write(&viewer.join("index.html"), "<h1>lamp</h1>");
    write(&data.join("index.json"), r#"{"generated_at": "x"}"#);
    (viewer, data)
}

#[test]
fn serves_viewer_and_data() {
    let dir = common::tmp();
    let (viewer, data) = layout(dir.path());
    let server = start(Viewer::Dir(viewer), &data);
    let root = get(server.port, "/");
    assert!(root.status == 200 && String::from_utf8_lossy(&root.body).contains("lamp"));
    let index = get(server.port, "/data/index.json");
    assert!(index.status == 200 && String::from_utf8_lossy(&index.body).contains("generated_at"));
}

#[test]
fn data_path_traversal_is_refused() {
    let dir = common::tmp();
    let (viewer, data) = layout(dir.path());
    write(&dir.path().join("outside.txt"), "must never be served");
    let server = start(Viewer::Dir(viewer), &data);
    for attempt in [
        "/data/../outside.txt",
        "/data/..%2foutside.txt",
        "/data/%2e%2e/outside.txt",
        "/data/sub/../../outside.txt",
        "/data/....//outside.txt",
        "/data/%252e%252e/outside.txt",
        "/%252e%252e/outside.txt",
    ] {
        let reply = get(server.port, attempt);
        assert_ne!(reply.status, 200, "{attempt}");
        assert!(
            !String::from_utf8_lossy(&reply.body).contains("must never be served"),
            "{attempt}"
        );
    }
    assert_eq!(get(server.port, "/data/index.json").status, 200);
}

#[test]
fn cache_headers_for_thumbs_only_on_success_and_on_revalidation() {
    let dir = common::tmp();
    let (viewer, data) = layout(dir.path());
    write(&data.join("thumbs/seller1/1.jpg"), b"jpg");
    let server = start(Viewer::Dir(viewer), &data);
    let thumb = get(server.port, "/data/thumbs/seller1/1.jpg?v=x");
    assert_eq!(
        (thumb.status, thumb.header("Cache-Control")),
        (200, Some("public, max-age=86400"))
    );
    let missing = get(server.port, "/data/thumbs/seller1/2.jpg");
    assert_eq!(
        (missing.status, missing.header("Cache-Control")),
        (404, Some("no-store"))
    );
    let index = get(server.port, "/data/index.json");
    assert_eq!(
        (index.status, index.header("Cache-Control")),
        (200, Some("no-store"))
    );
    let modified = thumb.header("Last-Modified").unwrap().to_string();
    let again = request(
        server.port,
        "GET",
        "/data/thumbs/seller1/1.jpg",
        &[("If-Modified-Since", &modified)],
    );
    assert_eq!(
        (again.status, again.header("Cache-Control")),
        (304, Some("public, max-age=86400"))
    );
}

#[test]
fn catalogue_shards_revalidate_by_content() {
    let dir = common::tmp();
    let (viewer, data) = layout(dir.path());
    let shard = data.join("catalog/seller.json");
    write(&shard, br#"{"value":1}"#);
    let mtime = std::fs::metadata(&shard).unwrap().modified().unwrap();
    let server = start(Viewer::Dir(viewer), &data);
    let first = get(server.port, "/data/catalog/seller.json");
    assert_eq!(
        (first.status, first.body.as_slice()),
        (200, &br#"{"value":1}"#[..])
    );
    assert_eq!(first.header("Cache-Control"), Some("no-cache"));
    assert_eq!(first.header("Content-Type"), Some("application/json"));
    let etag = first.header("ETag").unwrap().to_string();
    let head = request(server.port, "HEAD", "/data/catalog/seller.json", &[]);
    assert!(
        head.status == 200 && head.body.is_empty() && head.header("ETag") == Some(etag.as_str())
    );
    for validator in [etag.clone(), format!("\"unrelated\", W/{etag}"), "*".into()] {
        let reply = request(
            server.port,
            "GET",
            "/data/catalog/seller.json",
            &[("If-None-Match", &validator)],
        );
        assert!(reply.status == 304 && reply.body.is_empty());
        assert_eq!(reply.header("Cache-Control"), Some("no-cache"));
        assert_eq!(reply.header("ETag"), Some(etag.as_str()));
    }
    write(&shard, br#"{"value":2}"#);
    std::fs::File::options()
        .write(true)
        .open(&shard)
        .unwrap()
        .set_modified(mtime)
        .unwrap();
    let fresh = request(
        server.port,
        "GET",
        "/data/catalog/seller.json",
        &[
            ("If-None-Match", &etag),
            ("If-Modified-Since", "Fri, 01 Jan 2100 00:00:00 GMT"),
        ],
    );
    assert_eq!(
        (fresh.status, fresh.body.as_slice()),
        (200, &br#"{"value":2}"#[..])
    );
    assert_ne!(fresh.header("ETag"), Some(etag.as_str()));
    let missing = request(
        server.port,
        "GET",
        "/data/catalog/missing.json",
        &[("If-None-Match", "*")],
    );
    assert!(
        missing.status == 404
            && missing.header("Cache-Control") == Some("no-store")
            && missing.header("ETag").is_none()
    );
    let escaped = get(server.port, "/data/catalog/../index.json");
    assert!(
        escaped.status == 200
            && escaped.header("Cache-Control") == Some("no-store")
            && escaped.header("ETag").is_none()
    );
    let post = request(server.port, "POST", "/data/catalog/seller.json", &[]);
    assert!(
        post.status == 501
            && post.header("Cache-Control") == Some("no-store")
            && post.header("ETag").is_none()
    );
}

#[test]
fn directory_listings_and_null_bytes_get_404() {
    let dir = common::tmp();
    let (viewer, data) = layout(dir.path());
    write(&data.join("catalog/sellerx.json"), "{}");
    let server = start(Viewer::Dir(viewer), &data);
    for path in [
        "/data/catalog/",
        "/data/",
        "/data/....//x",
        "/data/%00index.json",
        "/%00index.html",
    ] {
        assert_eq!(get(server.port, path).status, 404, "{path}");
    }
    assert_eq!(get(server.port, "/data/catalog/sellerx.json").status, 200);
    assert_eq!(get(server.port, "/").status, 200);
}

#[test]
fn security_headers_on_every_response() {
    let dir = common::tmp();
    let (viewer, data) = layout(dir.path());
    let server = start(Viewer::Dir(viewer), &data);
    for path in ["/", "/data/index.json", "/missing"] {
        let reply = get(server.port, path);
        let csp = reply.header("Content-Security-Policy").unwrap_or("");
        assert!(
            csp.contains("default-src 'none'")
                && csp.contains("script-src 'self'")
                && csp.contains("manifest-src 'self'"),
            "{path}: {csp}"
        );
        assert_eq!(reply.header("X-Content-Type-Options"), Some("nosniff"));
    }
}

#[test]
fn the_viewer_declares_and_serves_the_pwa_manifest() {
    let html = common::read(&common::root().join("viewer/index.html"));
    assert!(html.contains("rel=\"manifest\"") && html.contains("href=\"manifest.json\""));
    let manifest = common::read_json(&common::root().join("viewer/manifest.json"));
    assert_eq!(common::obj(&manifest)["name"].py_str(), "DiogenesLamp");
    assert_eq!(common::obj(&manifest)["start_url"].py_str(), "./");
    let dir = common::tmp();
    let data = dir.path().join("data");
    write(&data.join("index.json"), "{}");
    let server = start(Viewer::Embedded, &data);
    let reply = get(server.port, "/manifest.json");
    assert_eq!(reply.status, 200);
    assert_eq!(
        common::obj(&common::json(&String::from_utf8_lossy(&reply.body)))["name"].py_str(),
        "DiogenesLamp"
    );
    assert!(
        reply
            .header("Content-Security-Policy")
            .unwrap()
            .contains("manifest-src 'self'")
    );
}

#[test]
fn the_refusal_sentinel_is_a_name_nothing_ever_writes() {
    for module in ["scrape.rs", "storage.rs"] {
        let source = common::read(&common::root().join("src").join(module));
        assert!(!source.contains(REFUSED), "{module} writes the sentinel");
    }
}

/// The wheel smoke check, for the binary: every asset index.html loads is
/// embedded and served from outside any source checkout.
#[test]
fn every_viewer_asset_is_embedded_and_served() {
    let html = common::read(&common::root().join("viewer/index.html"));
    let mut paths = vec!["index.html".to_string()];
    let re = regex::Regex::new(r#"<(script|link)\b([^>]*)>"#).unwrap();
    let attr = |tag: &str, name: &str| -> Option<String> {
        regex::Regex::new(&format!(r#"\b{name}="([^"]*)""#))
            .unwrap()
            .captures(tag)
            .map(|c| c[1].to_string())
    };
    for caps in re.captures_iter(&html) {
        let tag = &caps[2];
        let path = if &caps[1] == "script" {
            attr(tag, "src")
        } else if attr(tag, "rel").is_some_and(|r| {
            r.to_lowercase()
                .split_whitespace()
                .any(|x| x == "stylesheet" || x == "manifest")
        }) {
            attr(tag, "href")
        } else {
            None
        };
        if let Some(path) = path
            && !paths.contains(&path)
        {
            paths.push(path);
        }
    }
    assert!(paths.len() > 5);
    let dir = common::tmp();
    let server = start(Viewer::Embedded, &dir.path().join("empty-data"));
    for path in &paths {
        let reply = get(server.port, &format!("/{path}"));
        assert!(reply.status == 200 && !reply.body.is_empty(), "{path}");
    }
}

#[test]
fn the_embedded_viewer_matches_the_viewer_directory() {
    let mut on_disk: Vec<String> = std::fs::read_dir(common::root().join("viewer"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| {
            [".html", ".css", ".js", ".json"]
                .iter()
                .any(|s| n.ends_with(s))
        })
        .collect();
    on_disk.sort();
    let embedded: Vec<String> = lamp::viewer::FILES
        .iter()
        .map(|(n, _)| n.to_string())
        .collect();
    assert_eq!(embedded, on_disk);
    for (name, data) in lamp::viewer::FILES {
        assert_eq!(
            std::fs::read(common::root().join("viewer").join(name)).unwrap(),
            *data,
            "{name}"
        );
    }
}
