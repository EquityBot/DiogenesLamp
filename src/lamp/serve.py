"""Serve the viewer and the data directory on localhost."""

from __future__ import annotations

import webbrowser
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import unquote


class _Handler(SimpleHTTPRequestHandler):
    # A name the scraper never writes, under the data directory, used as the
    # target for any request we refuse: the handler answers 404 for it
    # because it does not exist. Returning the data directory itself would
    # have been served as a listing instead. Nothing may ever create a file
    # by this name under data/.
    REFUSED = ".refused-traversal"

    def __init__(self, *args, viewer_dir: Path, data_dir: Path, **kwargs):
        self._viewer_dir = viewer_dir
        self._data_dir = data_dir
        super().__init__(*args, directory=str(viewer_dir), **kwargs)

    def translate_path(self, path: str) -> str:
        clean = unquote(path.split("?", 1)[0].split("#", 1)[0])
        if "\x00" in clean:
            # unquote leaves a %00 as a literal NUL; open() then raises
            # ValueError (not OSError), which stdlib send_head does not
            # catch, and the handler thread dies with no reply. Route every
            # spelling of it at the refused name for a 404.
            return str(self._data_dir / self.REFUSED)
        if clean.startswith("/data/"):
            relative = clean[len("/data/"):]
            # Anything else resolve() rejects must not escape
            # translate_path either: same 404, handler stays alive.
            try:
                candidate = (self._data_dir / relative).resolve()
                if candidate.is_relative_to(self._data_dir.resolve()):
                    return str(candidate)
            except (OSError, ValueError):
                pass
            return str(self._data_dir / self.REFUSED)
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
        # Everything the page needs, nothing else: scraped HTML feeds card
        # content, so the viewer is one escaping mistake away from XSS -
        # this policy is the seatbelt behind the escaping. The favicon is
        # an inline data: SVG; thumbs come from this origin; scripts and
        # styles are same-origin files with no inline blocks.
        self.send_header(
            "Content-Security-Policy",
            "default-src 'none'; img-src 'self' data:; script-src 'self'; "
            "style-src 'self'; connect-src 'self'; base-uri 'none'; "
            "form-action 'none'",
        )
        # Stops a browser from sniffing a JSON shard or an HTML error page
        # into something executable.
        self.send_header("X-Content-Type-Options", "nosniff")
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
