"""Run with an installed wheel's interpreter, outside editable development."""

import os
import tempfile
import threading
import urllib.request
from html.parser import HTMLParser
from pathlib import Path

from lamp import cli
from lamp.serve import make_server


class ViewerAssets(HTMLParser):
    def __init__(self):
        super().__init__()
        self.paths = ["index.html"]

    def handle_starttag(self, tag, attrs):
        attributes = dict(attrs)
        path = None
        if tag == "script":
            path = attributes.get("src")
        elif tag == "link" and set((attributes.get("rel") or "").lower().split()) & {
            "stylesheet", "manifest"
        }:
            path = attributes.get("href")
        if path and path not in self.paths:
            self.paths.append(path)


def main():
    assets = Path(cli.__file__).resolve().parent / "viewer"
    assert assets.is_dir(), "installed wheel has no viewer assets"
    page = ViewerAssets()
    page.feed((assets / "index.html").read_text(encoding="utf-8"))
    with tempfile.TemporaryDirectory() as directory:
        previous = Path.cwd()
        os.chdir(directory)
        server = make_server(assets, Path(directory) / "empty-data", 0)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            for asset in page.paths:
                with urllib.request.urlopen(f"http://127.0.0.1:{server.server_port}/{asset}") as response:
                    assert response.status == 200 and response.read(), asset
        finally:
            server.shutdown()
            server.server_close()
            thread.join()
            os.chdir(previous)
    print(f"Wheel smoke passed: {len(page.paths)} viewer assets served outside the source checkout")


if __name__ == "__main__":
    main()
