"""Serve the viewer and the data directory on localhost."""

from __future__ import annotations

import webbrowser
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path


class _Handler(SimpleHTTPRequestHandler):
    def __init__(self, *args, viewer_dir: Path, data_dir: Path, **kwargs):
        self._viewer_dir = viewer_dir
        self._data_dir = data_dir
        super().__init__(*args, directory=str(viewer_dir), **kwargs)

    def translate_path(self, path: str) -> str:
        clean = path.split("?", 1)[0].split("#", 1)[0]
        if clean.startswith("/data/"):
            relative = clean[len("/data/"):]
            candidate = (self._data_dir / relative).resolve()
            if candidate.is_relative_to(self._data_dir.resolve()):
                return str(candidate)
            # Point a refused traversal at a name the scraper never writes,
            # so it answers 404. Returning the data directory instead would
            # have been served as a directory listing, and a path carrying a
            # null byte crashes the handler before it can reply.
            return str(self._data_dir / ".refused-traversal")
        return super().translate_path(path)

    def send_response(self, code: int, message: str | None = None) -> None:
        self._status = code
        super().send_response(code, message)

    def end_headers(self) -> None:
        # A full catalog holds tens of thousands of thumbs - letting the
        # browser cache them makes reloads near-instant. Thumb files are
        # overwritten in place on re-scrape, so the viewer cache-busts with
        # a version query; the day-long lifetime is safe because the URL
        # changes whenever the content can. Only responses that carry or
        # confirm the image qualify: a cached 404 would mask a thumb that
        # lands in a later scrape, and answering a revalidation with
        # no-store would tell the browser to drop what it already holds.
        # Everything else (index, shards, viewer files) stays uncached.
        if (self.path.startswith("/data/thumbs/")
                and getattr(self, "_status", 0) in (200, 304)):
            self.send_header("Cache-Control", "public, max-age=86400")
        else:
            self.send_header("Cache-Control", "no-store")
        super().end_headers()

    def list_directory(self, path):
        """Never serve a directory listing.

        The viewer is reached through its index.html and every data file by
        an explicit path, so a listing only ever exposes the catalog's file
        names - including to a request that reached a directory by an odd
        spelling such as /data/....//x, which Windows collapses onto the
        data directory itself.
        """
        self.send_error(404, "No permission to list directory")

    def log_message(self, *args) -> None:  # keep terminal quiet
        pass


def make_server(viewer_dir: Path, data_dir: Path, port: int) -> ThreadingHTTPServer:
    handler = partial(_Handler, viewer_dir=viewer_dir, data_dir=data_dir)
    return ThreadingHTTPServer(("127.0.0.1", port), handler)


def serve(viewer_dir: Path, data_dir: Path, port: int = 8000, open_browser: bool = True) -> None:
    server = make_server(viewer_dir, data_dir, port)
    url = f"http://127.0.0.1:{server.server_address[1]}/"
    print(f"serving on {url} (Ctrl+C to stop)")
    if open_browser:
        webbrowser.open(url)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        server.shutdown()
