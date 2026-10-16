"""Select the newest listing using the viewer's existing duplicate identities."""

from __future__ import annotations

import re
import unicodedata
from collections import Counter
from datetime import date
from urllib.parse import parse_qsl, quote, unquote, urljoin, urlsplit

from lamp.age import estimate_published
from lamp.links import _SHORTLINK_HOSTS
from lamp.schema import canonical_album, public_https_host


def item_keys(links: list[dict]) -> set[str]:
    keys = set()
    for link in links:
        try:
            url = urlsplit(link["url"].replace("\\", "/"))
            host = public_https_host(link["url"])
            # URL() removes dot segments, including encoded dots, before keying paths.
            parts = [unquote(p) if unquote(p) in (".", "..") else p for p in url.path.split("/")]
            # The /. prefix keeps a leading // inside the path during urljoin.
            path = urlsplit(urljoin("https://example.invalid/", "/." + "/".join(parts))).path
            path = quote(path, safe="/!$&'()*+,;=:@%[]-._~|")
        except (KeyError, TypeError, ValueError):
            continue
        platform = next((p for p in ("taobao", "tmall", "weidian", "1688")
                         if host == p + ".com" or host.endswith("." + p + ".com")), None)
        ids = {v for k, v in parse_qsl(url.query) if k.lower() in ("id", "itemid") and v}
        offer = re.fullmatch(r"/offer/([0-9]+)\.html", path) if platform == "1688" else None
        if offer:
            ids.add(offer[1])
        if platform and len(ids) == 1 and re.fullmatch(r"[0-9]{1,24}", next(iter(ids))):
            keys.add(f"{platform}:{next(iter(ids))}")
        elif host in _SHORTLINK_HOSTS and len(path) > 1:
            keys.add(f"url:{host}{path}")
    return keys


def duplicate_key(album: dict) -> str | None:
    keys = item_keys(album.get("links", []))
    if len(keys) == 1:
        return "i:" + next(iter(keys))
    title = "".join(ch for ch in unicodedata.normalize("NFKC", album.get("title") or "").lower()
                    if unicodedata.category(ch)[0] in "LN")
    if sum(2 if ord(ch) > 0x2e7f else 1 for ch in title) < 8:
        return None
    return f"t:{album['seller']}:{title}"


def newest_listings(albums: list[dict]) -> tuple[list[dict], dict[str, str]]:
    """Return canonical records and older-ID aliases, without changing captures.

    A title used over twice by one seller is a category label, as in dedup.js.
    Posting date wins, then album ID; fetch timestamps and link counts do not.
    """
    albums = [album for album in albums if canonical_album(album)]
    keys = [duplicate_key(album) for album in albums]
    counts = Counter(key for key in keys if key and key.startswith("t:"))
    winners: dict[str, dict] = {}
    for album, key in zip(albums, keys):
        if not key or counts[key] > 2:
            continue
        current = winners.get(key)
        if current is None or _rank(album) > _rank(current):
            winners[key] = album
    selected = []
    aliases = {}
    for album, key in zip(albums, keys):
        winner = winners.get(key) if key else None
        if winner is None or winner is album:
            selected.append(album)
        elif album["id"] != winner["id"]:
            aliases[album["id"]] = winner["id"]
    return selected, aliases


def _rank(album: dict) -> tuple[date, int]:
    album_id = int(album["id"])
    known = album.get("date_published") or ""
    if isinstance(known, str) and re.fullmatch(r"[0-9]{4}-[0-9]{2}-[0-9]{2}", known[:10]):
        try:
            return date.fromisoformat(known[:10]), album_id
        except ValueError:
            pass
    return estimate_published(album_id), album_id
