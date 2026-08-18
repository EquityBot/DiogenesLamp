import threading
import urllib.error
import urllib.request
from pathlib import Path

from lamp.serve import make_server


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
    """/data/ must not reach outside the data dir, however the path is spelled."""
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


def test_directory_listings_are_refused(tmp_path: Path):
    """A listing exposes the catalog's file names and is never needed: the
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
