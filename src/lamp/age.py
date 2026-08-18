"""Estimate an album's posting date from its id.

Yupoo assigns album ids from a global sequence, so an id dates an album even
before its page is fetched - which is what lets a scrape skip stale listings
without paying a request for each one. ANCHORS are (album id, publish date)
pairs measured live on 2026-08-18 from album pages spread across the id
range; interpolating between them is accurate to days.

viewer/age.js carries the same table for the browser side.
tests/test_age.py asserts the two stay identical.
"""

from __future__ import annotations

from datetime import date

ANCHORS: tuple[tuple[int, date], ...] = (
    (8348553, date(2017, 11, 7)),
    (70347667, date(2019, 7, 24)),
    (71628395, date(2019, 8, 24)),
    (119598315, date(2022, 8, 7)),
    (130174962, date(2023, 3, 2)),
    (160926298, date(2024, 4, 18)),
    (190225975, date(2025, 3, 27)),
    (220478978, date(2025, 12, 18)),
    (250756373, date(2026, 8, 17)),
)


def estimate_published(album_id: int) -> date:
    """Best estimate of when this album was posted.

    Clamps below the first anchor; past the last anchor it extrapolates at the
    final segment's rate, so ids minted after the table was measured still
    date sensibly.
    """
    if album_id <= ANCHORS[0][0]:
        return ANCHORS[0][1]
    index = next(
        (i for i, (anchor_id, _) in enumerate(ANCHORS) if anchor_id >= album_id),
        len(ANCHORS) - 1,
    )
    id0, date0 = ANCHORS[index - 1]
    id1, date1 = ANCHORS[index]
    span_days = (date1 - date0).days
    # Floor, not round: viewer/age.js interpolates in milliseconds and reads
    # the calendar date off the resulting instant, which truncates. Rounding
    # here would put the two languages a day apart in the middle of every
    # segment, so a 180-day scrape and a 180-day filter would disagree about
    # the same album. tests/test_age.py checks they still match.
    offset = (album_id - id0) * span_days // (id1 - id0)
    return date0.fromordinal(date0.toordinal() + offset)


def published_date(album_id: int, known: str = "") -> date:
    """Publish date for an album: a previously scraped date wins over the estimate."""
    if known:
        try:
            return date.fromisoformat(known[:10])
        except ValueError:
            pass
    return estimate_published(album_id)


def is_stale(album_id: int, max_age_days: int, today: date, known: str = "") -> bool:
    """True when this album was posted more than max_age_days ago.

    max_age_days <= 0 disables the check - nothing is stale.
    """
    if max_age_days <= 0:
        return False
    return (today - published_date(album_id, known)).days > max_age_days
