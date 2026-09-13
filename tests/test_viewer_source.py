"""Source-level contracts for the viewer, and file hygiene for the repo.

viewer/app.js is the DOM layer: it measures the page, builds cards and binds
listeners, so unlike the pure modules beside it there is no way to load it
under node. The guarantees it makes that no unit test can reach are pinned
here instead, in the same shape as the cross-file checks in test_links.py
and test_age.py.
"""

import re
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
APP_JS = ROOT / "viewer" / "app.js"

# Every suffix a person edits by hand. Binaries (docs/*.webp) are left out
# on purpose - a NUL and a CRLF are what a WebP is made of.
TEXT_SUFFIXES = frozenset({".py", ".js", ".cjs", ".json", ".toml", ".md",
                           ".css", ".html", ".yml", ".yaml", ".lock"})


def _tracked_files() -> list[Path]:
    """Every path git tracks, as absolute paths. -z so a path with a space,
    a quote or a non-ASCII character arrives whole rather than git-quoted."""
    listed = subprocess.run(
        ["git", "-C", str(ROOT), "ls-files", "-z"],
        capture_output=True, text=True, timeout=60, check=False,
    )
    assert listed.returncode == 0, (
        f"git ls-files failed in {ROOT}: {listed.stderr.strip()}"
    )
    return [ROOT / name for name in listed.stdout.split("\0") if name]


def _text_sources() -> list[Path]:
    """Hand-edited text files, taken from what the repository tracks.

    Walking the clone and filtering afterwards descended into every tree it
    meant to skip, .git included, and the skip list named none of the
    generated trees a normal checkout carries - dist/, build/, .pytest_cache/,
    .impeccable/, all of them in .gitignore. A generated .js, .json or .md
    written there with CRLF endings or a NUL byte failed the assertions below
    in a working tree git itself reports as clean.

    What git tracks is exactly the hand-edited set these assertions are about,
    and it excludes ignored and generated files by construction. A new file
    joins the set the moment it is `git add`ed, which is before it can reach a
    commit. Tracked paths deleted from the working tree are skipped.
    """
    return sorted(path for path in _tracked_files()
                  if path.suffix in TEXT_SUFFIXES and path.is_file())


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


def test_app_js_reaches_storage_only_through_the_guarded_accessors():
    """localStorage's accessors throw, not just the values they return.

    A browser with site data blocked for the origin raises SecurityError
    from getItem itself. app.js reads the favourites and the colour theme
    while its module body is still running, so a raw call there aborts the
    whole module: no listeners bound, no catalogue loaded, and a permanently
    blank grid recoverable only by clearing site data. filters.js owns the
    guarded readStored/writeStored that every access goes through.
    """
    source = APP_JS.read_text(encoding="utf-8")
    raw = re.findall(r"localStorage\.\w+", source)
    assert raw == [], f"unguarded storage access in app.js: {raw}"

    # Routing through the helpers is not by itself the guard. `window.
    # localStorage` is a getter and raises from the getter, and an argument is
    # evaluated at the call site - so `readStored(localStorage, key)` threw
    # before readStored's try was ever entered, and every one of these sites
    # still aborted the module exactly as it had before. The host is what
    # crosses the call, and filters.js reaches for the Storage inside its own
    # try.
    calls = re.findall(r"\b(readStored|writeStored|readFilters|writeFilters)"
                       r"\(([^,)]*)", source)
    assert calls, "app.js no longer routes storage through the guarded accessors"
    for name, first in calls:
        assert first.strip() == "window", (
            f"{name}() is handed {first.strip()!r}, which is evaluated at the "
            "call site, outside the guard's try - pass the host instead"
        )


def test_every_focusable_control_on_a_card_carries_the_roving_tabindex():
    """The roving tabindex is meant to be one tab stop for the whole lattice.

    A card is not a single focusable node: it carries a cover link, a star,
    sometimes a duplicate badge and up to 3 purchase links, each
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


def test_the_roving_tab_stop_follows_the_cursor_wherever_it_moves():
    """The one tab stop is stamped on the card under the cursor, so it has to
    move whenever the cursor does.

    Only the render loop stamped it, and most cursor moves never reach that
    loop. renderGrid returns early when the row range, the column count, the
    filtered length and the filter version are all unchanged - which is
    exactly an ArrowRight inside the visible row - and the focusin handler, a
    click on a card or a Tab into one, moved the cursor while rendering
    nothing at all. Both left tabindex="0" on the card the reader had just
    left, and Tab then jumped backwards to it, which is the one behaviour the
    roving tabindex exists to prevent.

    There is no DOM harness in this repo, so the contract is pinned at the
    source for DOM effects: setCursor stamps, focusin goes through it, and
    the render key carries the cursor. Pure transitions and cursor clamping
    are covered through view-state.js in the Node suite.
    """
    source = APP_JS.read_text(encoding="utf-8")

    setter = re.search(r"\nfunction setCursor\(next\) \{\n(.*?)\n\}\n", source, re.S)
    assert setter, "app.js no longer declares setCursor"
    assert "setCardTabStop(" in setter.group(1), \
        "setCursor moves the cursor without moving the tab stop"

    focusin = re.search(r'addEventListener\("focusin".*?\n\}\);', source, re.S)
    assert focusin, "app.js no longer binds focusin on the grid"
    assert "setCursor(" in focusin.group(0), (
        "focusin moves the cursor without moving the tab stop, and it renders "
        "nothing that would move it afterwards"
    )

    assert "cursor: state.view.cursor" in source, (
        "the render key omits the cursor, so renderGrid skips the very pass "
        "that would stamp the stop on the card the cursor moved to"
    )


def test_the_render_loop_refreshes_the_title_of_every_card_it_reuses():
    """A reused card kept the marks drawn for the previous query.

    fillTitle ran once, from makeCard. A changed query bumps the filter
    version, so renderGrid does run, but it reuses the cards already in the
    window and only re-places them. Searching "jacket" and then "jacket grid"
    therefore left cards showing the earlier mark alone, and clearing the
    query left the marks on every surviving card.

    The behaviour of refreshCardTitle is covered against an element double in
    tests/viewer/title-refresh.test.cjs. What no unit test reaches is whether
    the render loop calls it, so that is pinned here.
    """
    source = APP_JS.read_text(encoding="utf-8")

    loop = re.search(r"\nfunction renderGrid\(force = false\) \{\n(.*?)\n\}\n", source, re.S)
    assert loop, "app.js no longer declares renderGrid"
    reuse = re.search(r"let card = cardCache\.get\(key\);\n(.*?)\n    if \(!card\) \{", 
                      loop.group(1), re.S)
    assert reuse, "renderGrid no longer has a card reuse branch"
    assert "refreshCardTitle(" in reuse.group(1), (
        "renderGrid reuses a card without refreshing its title, so the marks "
        "from the previous query stay on it"
    )

    maker = re.search(r"\nfunction makeCard\(album\) \{\n(.*?)\n\}\n", source, re.S)
    assert maker, "app.js no longer declares makeCard"
    assert "refreshCardTitle(" in maker.group(1), (
        "makeCard fills the title without stamping the query it used, so the "
        "reuse branch cannot tell whether a refill is needed"
    )
