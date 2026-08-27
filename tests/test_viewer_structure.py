"""Structural accessibility contracts for the static viewer shell."""

import re
from pathlib import Path

from bs4 import BeautifulSoup


VIEWER_HTML = Path(__file__).resolve().parents[1] / "viewer" / "index.html"


def test_visible_brand_is_the_single_top_level_heading() -> None:
    soup = BeautifulSoup(VIEWER_HTML.read_text(encoding="utf-8"), "html.parser")
    headings = soup.find_all(re.compile(r"^h[1-6]$"))
    level_ones = soup.find_all("h1")

    assert len(level_ones) == 1
    assert level_ones[0].get("id") == "brand"
    assert " ".join(level_ones[0].stripped_strings) == "Diogenes Lamp"
    assert headings[0] == level_ones[0]
    assert soup.select_one("span#brand") is None
