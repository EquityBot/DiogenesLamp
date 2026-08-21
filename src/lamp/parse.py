"""Parsers for Yupoo album list pages and album pages."""

from __future__ import annotations

import json
import re
from dataclasses import dataclass

from bs4 import BeautifulSoup

_ALBUM_HREF_RE = re.compile(r"/albums/(\d+)")
_INT_RE = re.compile(r"\d+")
_DATE_RE = re.compile(r"^\d{4}-\d{2}-\d{2}")
_PRICE_PATTERNS = (
    re.compile(r"(?:¥|￥|\brmb|\byuan)\s*([0-9]{2,5})\b", re.IGNORECASE),
    re.compile(r"\b([0-9]{2,5})\s*(?:¥|￥|元|块|rmb|yuan)\b", re.IGNORECASE),
    re.compile(r"\b[pP]([0-9]{2,4})\b"),
    re.compile(r"\b([0-9]{2,4})[pP]\b"),
)


def parse_price(text: str) -> int:
    """Extract price in CNY/RMB from title or description text (e.g. ¥180, 180元, P180, 180p).

    Returns 0 if no price found or if the value is outside sensible catalogue ranges (10..99999).
    """
    for pat in _PRICE_PATTERNS:
        match = pat.search(text)
        if match:
            val = int(match.group(1))
            if 10 <= val <= 99999:
                return val
    return 0


@dataclass(frozen=True)
class AlbumStub:
    id: str
    title: str
    thumb_url: str
    photo_count: int


@dataclass(frozen=True)
class AlbumDetail:
    title: str
    description: str
    category: str
    photo_count: int
    date_published: str = ""
    date_modified: str = ""
    price: int = 0


def _soup(html: str) -> BeautifulSoup:
    return BeautifulSoup(html, "lxml")


def _absolute(url: str) -> str:
    if url.startswith("//"):
        return "https:" + url
    return url


def _first_int(text: str) -> int:
    match = _INT_RE.search(text or "")
    return int(match.group()) if match else 0


# Yupoo serves two list-page templates. The original names its cards
# album__*; a second one (live 2026-08, and the majority in a real link
# collection) names them album3__* and lazy-loads covers, putting a base64
# placeholder in src and the real URL in data-origin-src.
_CARD_SELECTOR = "a.album__main, a.album3__main"
_COUNT_SELECTOR = ".album__count, .album__photonumber"
_THUMB_ATTRS = ("data-src", "data-origin-src", "src")
# Cover images first, in the classes both templates use for them; a bare img
# is the last resort. Without this a badge or ribbon rendered before the
# cover would be downloaded as the album's thumbnail.
_THUMB_SELECTORS = ("img.album__img", "img.album3__img", "img")
# An album with no cover points at Yupoo's own "no image" asset. Downloading
# it gives every such album an identical grey placeholder card; treating it
# as no cover at all lets the viewer hide the image instead.
_PLACEHOLDER_COVERS = ("im_photo_album.png",)


def _is_placeholder(url: str) -> bool:
    return any(url.rsplit("/", 1)[-1] == name for name in _PLACEHOLDER_COVERS)


def _card_thumb(card) -> str:
    """Best cover URL in a card, across both templates.

    Skips inline data: placeholders (storing one would make every download
    fail) and Yupoo's no-cover asset, and keeps looking rather than giving
    up on the card - a placeholder in the first image does not mean a later
    one is not a real cover.
    """
    for selector in _THUMB_SELECTORS:
        for img in card.select(selector):
            for attr in _THUMB_ATTRS:
                value = (img.get(attr) or "").strip()
                if not value or value.startswith("data:") or _is_placeholder(value):
                    continue
                return _absolute(value)
    return ""


def parse_album_list(html: str) -> list[AlbumStub]:
    stubs: list[AlbumStub] = []
    for card in _soup(html).select(_CARD_SELECTOR):
        href_match = _ALBUM_HREF_RE.search(card.get("href", ""))
        if not href_match:
            continue
        title_el = card.select_one(".album__title")
        title = (card.get("title") or (title_el.get_text(strip=True) if title_el else "")).strip()
        # .album__count was absent from live album__ markup as of 2026-08;
        # the album3__ template carries the count in .album__photonumber.
        # Falls through to 0 via _first_int when neither is present.
        count_el = card.select_one(_COUNT_SELECTOR)
        stubs.append(
            AlbumStub(
                id=href_match.group(1),
                title=title,
                thumb_url=_card_thumb(card),
                photo_count=_first_int(count_el.get_text() if count_el else ""),
            )
        )
    return stubs


def parse_album_page(html: str) -> AlbumDetail:
    soup = _soup(html)
    title_el = soup.select_one(".showalbumheader__gallerytitle")
    title = (title_el.get("data-name") or title_el.get_text(strip=True)) if title_el else ""
    desc_el = soup.select_one(".showalbumheader__gallerysubtitle")
    description = desc_el.get_text("\n", strip=True) if desc_el else ""
    # .showalbumheader__categorieslink was absent from live album-page markup as
    # of 2026-08. That layout only exposes a seller-wide category nav
    # (a.showheader__link, every category, not this album's) - not wired up here
    # since it would misattribute a category. Kept for older/variant layouts;
    # falls through to "" when missing.
    cat_el = soup.select_one(".showalbumheader__categorieslink")
    category = cat_el.get_text(strip=True) if cat_el else ""
    # .showalbumheader__gallerycount was likewise absent from live markup as of
    # 2026-08 - kept for older/variant layouts; falls through to 0 when missing.
    count_el = soup.select_one(".showalbumheader__gallerycount")
    published, modified = _dates_from_ld_json(soup)
    price = parse_price(f"{title} {description}")
    return AlbumDetail(
        title=title.strip(),
        description=description,
        category=category,
        photo_count=_first_int(count_el.get_text() if count_el else ""),
        date_published=published,
        date_modified=modified,
        price=price,
    )


def _ld_json_objects(soup: BeautifulSoup):
    """Yield every JSON-LD object on the page.

    JSON-LD legitimately encodes the same data as a bare object, an array of
    objects, or an @graph wrapper, and a page may carry several blocks. Yield
    all of them so a date is found wherever it lives; malformed blocks are
    skipped rather than aborting the scan.
    """
    for script in soup.select('script[type="application/ld+json"]'):
        try:
            payload = json.loads(script.get_text())
        except (ValueError, TypeError):
            continue
        queue = [payload]
        while queue:
            node = queue.pop(0)
            if isinstance(node, list):
                queue.extend(node)
            elif isinstance(node, dict):
                yield node
                graph = node.get("@graph")
                if isinstance(graph, (list, dict)):
                    queue.append(graph)


def _dates_from_ld_json(soup: BeautifulSoup) -> tuple[str, str]:
    """Album pages embed JSON-LD with datePublished/dateModified (live 2026-08).

    Each date is taken from the first object that carries a well-formed value
    for it, independently: a block holding only dateModified must not stop a
    later block's datePublished from being found. Returns ("", "") when no
    block carries either - the viewer then estimates the posting date from
    the album id.
    """
    published = modified = ""
    for node in _ld_json_objects(soup):
        if not published:
            value = str(node.get("datePublished") or "")
            if _DATE_RE.match(value):
                published = value
        if not modified:
            value = str(node.get("dateModified") or "")
            if _DATE_RE.match(value):
                modified = value
        if published and modified:
            break
    return published, modified
