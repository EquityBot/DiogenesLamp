import json
import os
import threading
import urllib.error
import urllib.request
from pathlib import Path

from lamp.serve import make_server

ROOT = Path(__file__).resolve().parents[1]


def _get(port: int, path: str) -> tuple[int, bytes]:
    with urllib.request.urlopen(f"http://127.0.0.1:{port}{path}") as resp:
        return resp.status, resp.read()


def _cache_control(port: int, path: str) -> tuple[int, str | None]:
    try:
        with urllib.request.urlopen(f"http://127.0.0.1:{port}{path}") as resp:
            return resp.status, resp.headers.get("Cache-Control")
    except urllib.error.HTTPError as err:
        return err.code, err.headers.get("Cache-Control")


def test_serves_viewer_and_data(tmp_path: Path):
    viewer = tmp_path / "viewer"
    data = tmp_path / "data"
    viewer.mkdir()
    data.mkdir()
    (viewer / "index.html").write_text("<h1>lamp</h1>", encoding="utf-8")
    (data / "index.json").write_text('{"generated_at": "x"}', encoding="utf-8")

    server = make_server(viewer, data, port=0)
    port = server.server_address[1]
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        status, body = _get(port, "/")
        assert status == 200 and b"lamp" in body
        status, body = _get(port, "/data/index.json")
        assert status == 200 and b"generated_at" in body
    finally:
        server.shutdown()


def test_data_path_traversal_is_refused(tmp_path: Path):
    """/data/ must not reach outside the data dir, however the path is spelt."""
    viewer = tmp_path / "viewer"
    data = tmp_path / "data"
    viewer.mkdir()
    data.mkdir()
    (viewer / "index.html").write_text("<h1>lamp</h1>", encoding="utf-8")
    (data / "index.json").write_text("{}", encoding="utf-8")
    private = tmp_path / "outside.txt"
    private.write_text("must never be served", encoding="utf-8")

    server = make_server(viewer, data, port=0)
    port = server.server_address[1]
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        for attempt in (
            "/data/../outside.txt",
            "/data/..%2foutside.txt",
            "/data/%2e%2e/outside.txt",
            "/data/sub/../../outside.txt",
            "/data/....//outside.txt",
            # Double-encoded: this handler unquotes once, stdlib unquotes
            # again on the viewer route, so a payload that only becomes ".."
            # after the second pass must still be refused.
            "/data/%252e%252e/outside.txt",
            "/%252e%252e/outside.txt",
        ):
            status, body = 0, b""
            try:
                with urllib.request.urlopen(f"http://127.0.0.1:{port}{attempt}") as resp:
                    status, body = resp.status, resp.read()
            except urllib.error.HTTPError as err:
                status, body = err.code, err.read()
            # Assert on the outcome, not just the absence of the marker: a
            # regression serving a different file, or an empty 200, would
            # slip past a body-only check.
            assert status != 200, f"traversal returned 200: {attempt} ({status})"
            assert b"must never be served" not in body, f"traversal succeeded: {attempt}"

        # the guard must not have broken ordinary access
        assert _get(port, "/data/index.json")[0] == 200
    finally:
        server.shutdown()


def test_cache_headers_thumbs_only_and_only_on_success(tmp_path: Path):
    viewer = tmp_path / "viewer"
    data = tmp_path / "data"
    viewer.mkdir()
    (data / "thumbs" / "seller1").mkdir(parents=True)
    (viewer / "index.html").write_text("<h1>lamp</h1>", encoding="utf-8")
    (data / "index.json").write_text("{}", encoding="utf-8")
    (data / "thumbs" / "seller1" / "1.jpg").write_bytes(b"jpg")

    server = make_server(viewer, data, port=0)
    port = server.server_address[1]
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        # existing thumb: cacheable (query mirrors the viewer's ?v= busting)
        status, cache = _cache_control(port, "/data/thumbs/seller1/1.jpg?v=x")
        assert status == 200 and cache == "public, max-age=86400"
        # missing thumb: a cached 404 would mask a later successful download
        status, cache = _cache_control(port, "/data/thumbs/seller1/2.jpg")
        assert status == 404 and cache == "no-store"
        # data that changes between scrapes is never cached
        status, cache = _cache_control(port, "/data/index.json")
        assert status == 200 and cache == "no-store"
    finally:
        server.shutdown()


def test_revalidated_thumb_keeps_its_cache_header(tmp_path: Path):
    """A 304 answered with no-store would tell the browser to discard the
    copy it just revalidated, undoing the caching entirely."""
    viewer = tmp_path / "viewer"
    data = tmp_path / "data"
    viewer.mkdir()
    (data / "thumbs" / "s").mkdir(parents=True)
    (viewer / "index.html").write_text("x", encoding="utf-8")
    (data / "thumbs" / "s" / "1.jpg").write_bytes(b"jpg")

    server = make_server(viewer, data, port=0)
    port = server.server_address[1]
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        url = f"http://127.0.0.1:{port}/data/thumbs/s/1.jpg"
        with urllib.request.urlopen(url) as resp:
            last_modified = resp.headers["Last-Modified"]
        request = urllib.request.Request(url, headers={"If-Modified-Since": last_modified})
        try:
            with urllib.request.urlopen(request) as resp:
                status, cache = resp.status, resp.headers.get("Cache-Control")
        except urllib.error.HTTPError as err:
            status, cache = err.code, err.headers.get("Cache-Control")
        assert status == 304
        assert cache == "public, max-age=86400"
    finally:
        server.shutdown()


def test_catalogue_shards_revalidate_by_content(tmp_path: Path):
    viewer = tmp_path / "viewer"
    data = tmp_path / "data"
    viewer.mkdir()
    (data / "catalog").mkdir(parents=True)
    shard = data / "catalog" / "seller.json"
    shard.write_bytes(b'{"value":1}')
    (data / "index.json").write_bytes(b"{}")
    original_stat = shard.stat()
    server = make_server(viewer, data, port=0)
    port = server.server_address[1]
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()

    def request(headers=None, name="seller.json", method="GET"):
        req = urllib.request.Request(
            f"http://127.0.0.1:{port}/data/catalog/{name}",
            headers=headers or {},
            method=method,
        )
        try:
            response = urllib.request.urlopen(req)
        except urllib.error.HTTPError as error:
            response = error
        with response:
            return response.code, response.headers, response.read()

    try:
        status, headers, body = request()
        assert status == 200 and body == b'{"value":1}'
        assert headers["Cache-Control"] == "no-cache"
        assert headers["Content-Type"] == "application/json"
        etag = headers["ETag"]
        assert etag
        status, headers, body = request(method="HEAD")
        assert status == 200 and body == b"" and headers["ETag"] == etag
        for validator in (etag, f'"unrelated", W/{etag}', "*"):
            status, headers, body = request({"If-None-Match": validator})
            assert status == 304 and body == b""
            assert headers["Cache-Control"] == "no-cache"
            assert headers["ETag"] == etag

        # A same-size replacement with the same timestamp must still be fresh.
        shard.write_bytes(b'{"value":2}')
        os.utime(shard, ns=(original_stat.st_atime_ns, original_stat.st_mtime_ns))
        status, headers, body = request(
            {"If-None-Match": etag, "If-Modified-Since": "Fri, 01 Jan 2100 00:00:00 GMT"}
        )
        assert status == 200 and body == b'{"value":2}'
        assert headers["ETag"] != etag
        assert headers["Cache-Control"] == "no-cache"

        status, headers, body = request({"If-None-Match": "*"}, "missing.json")
        assert status == 404 and headers["Cache-Control"] == "no-store"
        assert headers["ETag"] is None
        status, headers, body = request(name="../index.json")
        assert status == 200 and headers["Cache-Control"] == "no-store"
        assert headers["ETag"] is None
        status, headers, body = request(method="POST")
        assert status == 501 and headers["Cache-Control"] == "no-store"
        assert headers["ETag"] is None
    finally:
        server.shutdown()


def test_directory_listings_are_refused(tmp_path: Path):
    """A listing exposes the catalogue's file names and is never needed: the
    viewer loads index.html and every data file by explicit path."""
    viewer = tmp_path / "viewer"
    data = tmp_path / "data"
    viewer.mkdir()
    (data / "catalog").mkdir(parents=True)
    (viewer / "index.html").write_text("<h1>lamp</h1>", encoding="utf-8")
    (data / "catalog" / "sellerx.json").write_text("{}", encoding="utf-8")

    server = make_server(viewer, data, port=0)
    port = server.server_address[1]
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        for path in ("/data/catalog/", "/data/", "/data/....//x"):
            status, _ = _cache_control(port, path)
            assert status == 404, f"{path} returned {status}"
        # the files themselves are still served
        assert _get(port, "/data/catalog/sellerx.json")[0] == 200
        assert _get(port, "/")[0] == 200
    finally:
        server.shutdown()


def test_null_byte_path_gets_404_not_a_dead_handler(tmp_path: Path):
    """resolve() raises on a null byte, and that exception used to escape
    translate_path and kill the handler thread with no reply at all. The
    answer must be a 404, and the server must keep answering afterwards."""
    viewer = tmp_path / "viewer"
    data = tmp_path / "data"
    viewer.mkdir()
    data.mkdir()
    (viewer / "index.html").write_text("<h1>lamp</h1>", encoding="utf-8")
    (data / "index.json").write_text("{}", encoding="utf-8")

    server = make_server(viewer, data, port=0)
    port = server.server_address[1]
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        # under /data/ (our handler) and on a viewer path (stdlib
        # send_head) alike: a NUL must come back as 404, not kill the
        # handler thread.
        for path in ("/data/%00index.json", "/%00index.html"):
            status, _ = _cache_control(port, path)
            assert status == 404, f"{path} returned {status}"
        # handler survived: ordinary requests still work
        assert _get(port, "/data/index.json")[0] == 200
        assert _get(port, "/")[0] == 200
    finally:
        server.shutdown()


def test_security_headers_on_every_response(tmp_path: Path):
    """Scraped HTML feeds card markup, so the viewer is one escaping
    mistake away from XSS. The CSP is the seatbelt behind the escaping, and
    it must be present on data files and errors too, not just the page."""
    viewer = tmp_path / "viewer"
    data = tmp_path / "data"
    viewer.mkdir()
    data.mkdir()
    (viewer / "index.html").write_text("<h1>lamp</h1>", encoding="utf-8")
    (data / "index.json").write_text("{}", encoding="utf-8")

    server = make_server(viewer, data, port=0)
    port = server.server_address[1]
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        for path in ("/", "/data/index.json", "/missing"):
            try:
                with urllib.request.urlopen(f"http://127.0.0.1:{port}{path}") as resp:
                    headers = resp.headers
            except urllib.error.HTTPError as err:
                headers = err.headers
            csp = headers.get("Content-Security-Policy", "")
            assert "default-src 'none'" in csp, f"{path}: no CSP ({csp})"
            assert "script-src 'self'" in csp, f"{path}: scripts not self-only"
            assert "manifest-src 'self'" in csp, f"{path}: PWA manifest blocked ({csp})"
            assert headers.get("X-Content-Type-Options") == "nosniff"
    finally:
        server.shutdown()


def test_viewer_declares_and_serves_the_pwa_manifest(tmp_path: Path):
    """The installable-app claim is the index.html link plus a JSON file the
    CSP actually allows. A missing file or a page that never references it
    would make lamp serve look fine while the browser never installs."""
    html = (ROOT / "viewer" / "index.html").read_text(encoding="utf-8")
    assert 'rel="manifest"' in html
    assert 'href="manifest.json"' in html
    on_disk = json.loads((ROOT / "viewer" / "manifest.json").read_text(encoding="utf-8"))
    assert on_disk["name"] == "DiogenesLamp"
    assert on_disk["start_url"] == "./"

    data = tmp_path / "data"
    data.mkdir()
    (data / "index.json").write_text("{}", encoding="utf-8")
    server = make_server(ROOT / "viewer", data, port=0)
    port = server.server_address[1]
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        status, body = _get(port, "/manifest.json")
        assert status == 200
        served = json.loads(body.decode())
        assert served["name"] == "DiogenesLamp"
        csp = urllib.request.urlopen(
            f"http://127.0.0.1:{port}/manifest.json"
        ).headers.get("Content-Security-Policy", "")
        assert "manifest-src 'self'" in csp
    finally:
        server.shutdown()


def test_the_refusal_sentinel_is_a_name_nothing_ever_writes():
    """Refused requests are pointed at data/<REFUSED> so they 404 by not
    existing. A writer that ever produced that name would turn every
    refused traversal into a served file."""
    from lamp import scrape, storage
    from lamp.serve import _Handler

    for module in (scrape, storage):
        source = Path(module.__file__).read_text(encoding="utf-8")
        assert _Handler.REFUSED not in source, f"{module.__name__} writes the sentinel"
