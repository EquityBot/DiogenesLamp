import hashlib
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def test_viewer_screenshot_has_current_provenance() -> None:
    screenshot = ROOT / "docs" / "viewer.webp"
    readme = (ROOT / "README.md").read_text(encoding="utf-8")
    product = (ROOT / "PRODUCT.md").read_text(encoding="utf-8")

    assert hashlib.sha256(screenshot.read_bytes()).hexdigest() in readme
    for value in (
        "docs/viewer.webp",
        "tests/make_demo_data.py",
        "MIT",
        "1280x800",
        "capture tooling was not retained",
    ):
        assert value in readme
    assert "two albums in every three" in " ".join(readme.lower().split())
    assert "two albums in every three" in " ".join(product.lower().split())
