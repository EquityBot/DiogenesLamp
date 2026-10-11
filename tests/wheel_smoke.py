"""Run with an installed wheel's interpreter, outside editable development."""

import os
import tempfile
import threading
import urllib.request
from pathlib import Path

from lamp import cli

# Import the scraper to exercise the wheel's runtime dependencies.
from lamp.scrape import scrape_all
from lamp.serve import make_server


def main():
    if not callable(scrape_all):
        raise AssertionError("installed wheel has no callable scraper entry point")
    assets = Path(cli.__file__).resolve().parent / "viewer"
    if not assets.is_dir():
        raise AssertionError("installed wheel has no viewer assets")
    with tempfile.TemporaryDirectory() as directory:
        previous = Path.cwd()
        os.chdir(directory)
        server = make_server(assets, Path(directory) / "empty-data", 0)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            for asset in ("index.html", "app.js", "saved-work.js", "style.css",
                          "user-store.js", "discovery.js"):
                with urllib.request.urlopen(f"http://127.0.0.1:{server.server_port}/{asset}") as response:
                    if response.status != 200 or len(response.read()) <= 100:
                        raise AssertionError(asset)
        finally:
            server.shutdown()
            server.server_close()
            thread.join()
            os.chdir(previous)
    print("Wheel smoke passed: 6 viewer assets served outside the source checkout")


if __name__ == "__main__":
    main()
