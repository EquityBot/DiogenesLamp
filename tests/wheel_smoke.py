"""Run with an installed wheel's interpreter, outside editable development."""

import os
import tempfile
import threading
import urllib.request
from pathlib import Path

from lamp import cli
from lamp.serve import make_server


def main():
    assets = Path(cli.__file__).resolve().parent / "viewer"
    assert assets.is_dir(), "installed wheel has no viewer assets"
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
                    assert response.status == 200 and len(response.read()) > 100, asset
        finally:
            server.shutdown()
            server.server_close()
            thread.join()
            os.chdir(previous)
    print("Wheel smoke passed: 6 viewer assets served outside the source checkout")


if __name__ == "__main__":
    main()
