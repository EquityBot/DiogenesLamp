"""Source-level contracts for the viewer, and file hygiene for the repo.

viewer/app.js is the DOM layer: it measures the page, builds cards and binds
listeners, so unlike the pure modules beside it there is no way to load it
under node. The guarantees it makes that no unit test can reach are pinned
here instead, in the same shape as the cross-file checks in test_links.py
and test_age.py.
"""

import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
APP_JS = ROOT / "viewer" / "app.js"

# Every suffix a person edits by hand. Binaries (docs/*.webp) are left out
# on purpose - a NUL and a CRLF are what a WebP is made of.
TEXT_SUFFIXES = frozenset({".py", ".js", ".cjs", ".json", ".toml", ".md",
                           ".css", ".html", ".yml", ".yaml", ".lock"})
SKIP_DIRS = frozenset({".git", ".ruff_cache", "__pycache__", "data",
                       "demo-data", "node_modules", ".venv", "venv"})


def _text_sources() -> list[Path]:
    """Hand-edited text files in the clone, generated trees excluded."""
    found = []
    for path in ROOT.rglob("*"):
        if not path.is_file() or path.suffix not in TEXT_SUFFIXES:
            continue
        parts = path.relative_to(ROOT).parts
        if any(part in SKIP_DIRS or part.endswith(".egg-info") for part in parts):
            continue
        found.append(path)
    return sorted(found)


def test_source_files_carry_no_nul_bytes():
    """A literal NUL turns a text file binary for the tools that read it.

    app.js held one as the separator in its card key, and both grep and
    ripgrep answer "binary file matches" for the whole 1149-line file -
    printing no lines at all in the file that holds nearly every piece of
    viewer logic. links.py writes its own control characters as escapes and
    says why; so does app.js now.
    """
    assert _text_sources(), "found no source files to check - has the layout moved?"
    for path in _text_sources():
        assert b"\x00" not in path.read_bytes(), f"{path.relative_to(ROOT)} holds a NUL byte"


def test_line_endings_match_the_editorconfig():
    """.editorconfig asks for LF everywhere, and app.js was the one file
    that did not comply. The first save in any editor honouring it rewrote
    all 1149 lines, so a one-line change arrived as a whole-file diff."""
    assert "end_of_line = lf" in (ROOT / ".editorconfig").read_text(encoding="utf-8")
    for path in _text_sources():
        assert b"\r\n" not in path.read_bytes(), f"{path.relative_to(ROOT)} uses CRLF"


def test_app_js_reaches_storage_only_through_the_guarded_accessors():
    """localStorage's accessors throw, not just the values they return.

    A browser with site data blocked for the origin raises SecurityError
    from getItem itself. app.js reads the favourites and the colour theme
    while its module body is still running, so a raw call there aborts the
    whole module: no listeners bound, no catalog loaded, and a permanently
    blank grid recoverable only by clearing site data. filters.js owns the
    guarded readStored/writeStored that every access goes through.
    """
    raw = re.findall(r"localStorage\.\w+", APP_JS.read_text(encoding="utf-8"))
    assert raw == [], f"unguarded storage access in app.js: {raw}"


def test_every_focusable_control_on_a_card_carries_the_roving_tabindex():
    """The roving tabindex is meant to be one tab stop for the whole lattice.

    A card is not a single focusable node: it carries a cover link, a star,
    sometimes a duplicate badge and up to three purchase links, each
    focusable in its own right. Stamping the tabindex on the card alone left
    every one of them in the tab order, and the render window holds roughly
    54 cards - several hundred stops to tab past a grid that advertises one.
    """
    source = APP_JS.read_text(encoding="utf-8")
    selector = re.search(r'CARD_STOPS = "([^"]+)"', source)
    assert selector, "app.js no longer declares CARD_STOPS"
    stops = {part.strip() for part in selector.group(1).split(",")}
    # Controls makeCard writes with a fixed class.
    built = set(re.findall(r'<(?:a|button) class="([a-z0-9-]+)"', source))
    assert built, "found no statically classed card controls - has makeCard changed?"
    for css_class in built:
        assert f".{css_class}" in stops, f".{css_class} is focusable but not a roving stop"
    # The purchase links take the platform as their class, so they are
    # covered by descendant rather than by name.
    assert ".card-links a" in stops
