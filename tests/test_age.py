import json
import re
import shutil
import subprocess
from datetime import UTC, date, datetime, timedelta
from itertools import pairwise
from pathlib import Path

import pytest

from lamp.age import ANCHORS, estimate_published, is_stale, published_date

VIEWER_AGE_JS = Path(__file__).resolve().parents[1] / "viewer" / "age.js"


def test_anchors_are_strictly_increasing():
    for (id0, date0), (id1, date1) in pairwise(ANCHORS):
        assert id1 > id0
        assert date1 > date0


def test_python_and_viewer_anchor_tables_match():
    """The two tables must agree - but see the estimate test below.

    Matching anchors alone does not prove matching dates: the interpolation
    between them is written twice, once per language.
    """
    source = VIEWER_AGE_JS.read_text(encoding="utf-8")
    block = re.search(r"AGE_ANCHORS = \[(.*?)\];", source, re.DOTALL).group(1)
    js_anchors = [
        (int(album_id), date(int(year), int(month) + 1, int(day)))  # JS months are 0-based
        for album_id, year, month, day in re.findall(
            r"\[(\d+),\s*Date\.UTC\((\d+),\s*(\d+),\s*(\d+)\)\]", block
        )
    ]
    assert js_anchors == list(ANCHORS)


def test_estimate_hits_anchors_and_interpolates():
    for album_id, published in ANCHORS:
        assert estimate_published(album_id) == published
    id0, date0 = ANCHORS[3]
    id1, date1 = ANCHORS[4]
    midpoint = estimate_published((id0 + id1) // 2)
    assert date0 < midpoint < date1


def test_estimate_clamps_below_and_extrapolates_above():
    assert estimate_published(1) == ANCHORS[0][1]
    last_id, last_date = ANCHORS[-1]
    assert estimate_published(last_id + 1_000_000) > last_date


def test_published_date_prefers_a_known_date():
    assert published_date(8348553, "2026-01-15") == date(2026, 1, 15)
    assert published_date(8348553, "2026-01-15T10:00:00+10:00") == date(2026, 1, 15)
    # unusable stored value falls back to the estimate
    assert published_date(8348553, "unknown") == ANCHORS[0][1]
    assert published_date(8348553) == ANCHORS[0][1]


def test_is_stale_uses_cutoff_and_known_dates():
    today = date(2026, 8, 18)
    old_id = ANCHORS[0][0]      # 2017
    fresh_id = ANCHORS[-1][0]   # yesterday
    assert is_stale(old_id, 180, today) is True
    assert is_stale(fresh_id, 180, today) is False
    # a known recent date rescues an album whose id looks ancient
    assert is_stale(old_id, 180, today, known="2026-08-01") is False
    # a known ancient date condemns an album whose id looks fresh
    assert is_stale(fresh_id, 180, today, known="2017-01-01") is True
    # 0 disables the gate entirely
    assert is_stale(old_id, 0, today) is False


def test_is_stale_boundary_matches_the_viewer_window():
    """An album exactly N days old is kept, as in the viewer's age filter."""
    today = date(2026, 8, 18)
    album_id, published = ANCHORS[-2]  # 2025-12-18, comfortably older than a day
    days_old = (today - published).days
    assert is_stale(album_id, days_old, today) is False
    assert is_stale(album_id, days_old - 1, today) is True


@pytest.mark.skipif(shutil.which("node") is None, reason="node not installed")
def test_python_and_viewer_estimates_agree_between_anchors():
    """The scraper decides what to fetch, the viewer what to show.

    Both estimate an album's date from its id, in different languages. If the
    interpolation drifts, a 180-day scrape and a 180-day filter disagree
    about the same album - so compare the dates themselves, not just the
    anchor tables they are built from.
    """
    probes = []
    for (id0, _), (id1, _) in pairwise(ANCHORS):
        span = id1 - id0
        probes += [id0 + span // 4, id0 + span // 2, id0 + (3 * span) // 4]
    probes += [1, ANCHORS[0][0] - 1, ANCHORS[-1][0] + 5_000_000]

    script = (
        f"const {{ estimatePublishedMs }} = require({json.dumps(str(VIEWER_AGE_JS))});\n"
        f"const ids = {json.dumps(probes)};\n"
        "console.log(JSON.stringify(ids.map((id) =>\n"
        "  new Date(estimatePublishedMs(id)).toISOString().slice(0, 10))));\n"
    )
    result = subprocess.run(["node", "-e", script], capture_output=True, text=True,
                            timeout=60, check=True)
    js_dates = json.loads(result.stdout)
    py_dates = [estimate_published(i).isoformat() for i in probes]

    mismatches = [(i, p, j) for i, p, j in zip(probes, py_dates, js_dates, strict=True)
                  if p != j]
    assert not mismatches, f"python/js date estimates diverge: {mismatches[:5]}"


@pytest.mark.skipif(shutil.which("node") is None, reason="node not installed")
def test_python_and_viewer_agree_on_the_keep_decision():
    """Matching dates are not enough - the cutoff comparison must match too.

    The scraper compares calendar dates while the viewer works in
    timestamps, so an album exactly at the cutoff can be fetched by
    --max-age-days N and then hidden by the N-day filter unless both count
    whole days from the same reference.
    """
    today = date(2026, 8, 18)
    # Local mid-afternoon, matching production: the scraper stamps runs with
    # a local timestamp and the viewer reads a local Date.now(). Building
    # this in UTC would put the two on different calendar days wherever the
    # machine is not near UTC, and the test would fail for that reason
    # rather than the one it is checking. The time of day matters: a
    # millisecond comparison against "N days ago" lands after midnight of
    # the boundary date and drops an album the scraper kept.
    now_ms = int(
        datetime(today.year, today.month, today.day, 15, 30).astimezone().timestamp() * 1000
    )
    windows = [30, 90, 180, 365]
    # Dates pinned exactly on, either side of, and far from each boundary.
    # Interpolated ids almost never land on a cutoff, so probing by id alone
    # would pass whatever the comparison does - the disagreement only shows
    # up at the boundary itself.
    cases = []
    for window in windows:
        for offset in (window - 1, window, window + 1, 0, window + 40):
            cases.append((window, (today - timedelta(days=offset)).isoformat(), offset))

    script = (
        f"const {{ withinAgeWindow }} = require({json.dumps(str(VIEWER_AGE_JS))});\n"
        f"const cases = {json.dumps([[w, d] for w, d, _ in cases])};\n"
        f"const now = {now_ms};\n"
        "console.log(JSON.stringify(cases.map(([w, d]) =>\n"
        "  withinAgeWindow({ id: '250000000', date_published: d }, w, now))));\n"
    )
    result = subprocess.run(["node", "-e", script], capture_output=True, text=True,
                            timeout=60, check=True)
    js_keep = json.loads(result.stdout)

    disagreements = []
    for (window, published, offset), kept_by_js in zip(cases, js_keep, strict=True):
        kept_by_python = not is_stale(250000000, window, today, known=published)
        if kept_by_python != kept_by_js:
            disagreements.append((window, offset, kept_by_python, kept_by_js))
    assert not disagreements, f"scrape/filter disagree: {disagreements[:5]}"

    # and the rule itself is the documented one: exactly N days old is kept
    at_cutoff = (today - timedelta(days=180)).isoformat()
    assert is_stale(250000000, 180, today, known=at_cutoff) is False
