"""Small validation boundary for local catalogue and recovery files."""

from __future__ import annotations

import ipaddress
import math
import os
import re
from pathlib import Path
from urllib.parse import unquote, urlsplit

import idna


def _numeric_host(host: str) -> str:
    """Apply the URL Standard's numeric-host admission, including short IPv4 forms."""
    parts = host.removesuffix(".").split(".")
    if not re.fullmatch(r"[0-9]+|0x[0-9a-f]*", parts[-1]):
        return host
    if len(parts) > 4:
        raise ValueError("invalid IPv4 hostname")
    numbers = []
    for part in parts:
        if not part:
            raise ValueError("invalid IPv4 hostname")
        radix, digits = (16, part[2:]) if part.startswith("0x") else (
            (8, part[1:]) if len(part) > 1 and part.startswith("0") else (10, part))
        pattern = {16: r"[0-9a-f]*", 8: r"[0-7]*", 10: r"[0-9]+"}[radix]
        if not re.fullmatch(pattern, digits):
            raise ValueError("invalid IPv4 hostname")
        numbers.append(int(digits, radix) if digits else 0)
    if any(n > 255 for n in numbers[:-1]) or numbers[-1] >= 256 ** (5 - len(numbers)):
        raise ValueError("invalid IPv4 hostname")
    address = numbers[-1] + sum(n * 256 ** (3 - i) for i, n in enumerate(numbers[:-1]))
    return str(ipaddress.IPv4Address(address))


def public_https_host(raw: str) -> str:
    """Admit canonical links without fetching or altering historical captures."""
    parsed = urlsplit(raw.replace("\\", "/"))
    if parsed.scheme != "https" or not parsed.hostname or parsed.username is not None or parsed.password is not None:
        raise ValueError("canonical links require public HTTPS without credentials")
    _ = parsed.port
    host = unquote(parsed.hostname, errors="strict").lower()
    if parsed.netloc.startswith("["):
        if "%" in host:
            raise ValueError("IPv6 zone identifiers are not canonical URL hosts")
        return str(ipaddress.IPv6Address(host))
    if any(ord(ch) <= 32 or ord(ch) == 127 or ch in "#/:<>?@[\\]^|%" for ch in host):
        raise ValueError("invalid canonical hostname")
    # Validate before mapping: NFKC alone would turn disallowed dot lookalikes
    # into valid separators and let them compete through the title fallback.
    try:
        if not host.isascii() or any(label.startswith("xn--") for label in host.split(".")):
            host = idna.encode(host, uts46=True).decode("ascii")
        if not host or host.startswith(("*", ".")):
            raise ValueError("invalid canonical hostname")
        return _numeric_host(host)
    except (UnicodeError, idna.IDNAError) as exc:
        raise ValueError("invalid canonical hostname") from exc


def canonical_album(album: dict) -> bool:
    """Keep legacy reading permissive; only admitted records compete for selection."""
    try:
        validate_metadata({"seller": album["seller"], "albums": [album]}, f"catalog/{album['seller']}.json")
        for raw in [album["album_url"], *(link["url"] for link in album["links"])]:
            public_https_host(raw)
        return True
    except (KeyError, TypeError, ValueError):
        return False


def data_path(root: Path, relative: str, kind: str = "metadata") -> Path:
    pattern = (r"thumbs/[a-z0-9_-]+/[0-9]+\.(?:jpg|png|webp)" if kind == "cover" else
               r"(?:index\.json|(?:catalog|runs)/[a-z0-9_-]+\.json)")
    if not isinstance(relative, str) or not re.fullmatch(pattern, relative):
        raise ValueError("unsafe catalogue path")
    root = root.resolve()
    path = Path(os.path.normpath(root / relative))
    if not path.is_relative_to(root):
        raise ValueError("catalogue path escapes data directory")
    # Resolve only the deepest entry that exists: a linked catalog/ or runs/
    # still fails containment, while the missing tail below it is created later
    # by this process as plain directories. Resolving the missing target raced
    # with a sibling seller thread creating catalog/ (Windows realpath keeps the
    # extended-length prefix when a component appears mid-call).
    existing = path
    while existing != root and not os.path.lexists(existing):
        existing = existing.parent
    if not existing.resolve().is_relative_to(root):
        raise ValueError("catalogue path escapes data directory")
    return path


def validate_metadata(value: object, relative: str) -> dict:
    if (not isinstance(value, dict) or type(value.get("schema_version", 1)) is not int
            or value.get("schema_version", 1) != 1):
        raise ValueError("unsupported metadata shape or schema version")
    if relative == "index.json":
        if "selection_version" in value and (type(value["selection_version"]) is not int or value["selection_version"] != 1):
            raise ValueError("unsupported selection version")
        sellers = value.get("sellers")
        if not isinstance(sellers, list):
            raise ValueError("index sellers must be an array")
        seen = set()
        indexed_ids: set[str] = set()
        for seller in sellers:
            if (not isinstance(seller, dict) or not isinstance(seller.get("id"), str)
                    or not re.fullmatch(r"[a-z0-9_-]+", seller["id"])):
                raise ValueError("invalid seller ID")
            if seller["id"] in seen or seller.get("shard") != f"catalog/{seller['id']}.json":
                raise ValueError("duplicate seller or mismatched shard path")
            seen.add(seller["id"])
            if "album_ids" in seller:
                ids = seller["album_ids"]
                if (not isinstance(ids, list) or any(not isinstance(i, str) or
                        not re.fullmatch(r"[0-9]{1,24}", i) for i in ids) or len(set(ids)) != len(ids)):
                    raise ValueError("invalid indexed album IDs")
                if indexed_ids.intersection(ids):
                    raise ValueError("duplicate indexed album ID across sellers")
                indexed_ids.update(ids)
            elif value.get("selection_version") == 1:
                raise ValueError("selected index requires album IDs for every seller")
        aliases = value.get("album_aliases", {})
        if not isinstance(aliases, dict) or any(
                not isinstance(old, str) or not re.fullmatch(r"[0-9]{1,24}", old) or
                not isinstance(new, str) or new not in indexed_ids or old in indexed_ids
                for old, new in aliases.items()):
            raise ValueError("invalid album aliases")
        return value
    if relative.startswith("runs/"):
        ids = value.get("failed_ids", [])
        if not isinstance(ids, list) or any(not isinstance(i, str) or not i.isdigit() for i in ids):
            raise ValueError("invalid failed album IDs")
        for field in ("last_attempt", "last_success", "last_listing"):
            if field in value and not isinstance(value[field], str):
                raise ValueError(f"invalid {field} timestamp")
        return value
    seller = Path(relative).stem
    if value.get("seller") != seller:
        raise ValueError("shard seller does not match its path")
    albums = value.get("albums")
    if not isinstance(albums, list):
        raise ValueError("shard albums must be an array")
    seen = set()
    for album in albums:
        if not isinstance(album, dict) or not re.fullmatch(r"[0-9]{1,24}", str(album.get("id", ""))):
            raise ValueError("invalid album ID")
        if not isinstance(album["id"], str) or album["id"] in seen:
            raise ValueError("duplicate or non-string album ID")
        seen.add(album["id"])
        if album.get("seller") != seller:
            raise ValueError("album seller does not match its shard")
        for field in ("title", "thumb", "album_url"):
            if not isinstance(album.get(field), str):
                raise ValueError(f"album {field} must be text")
        for field in ("category", "description", "first_seen", "updated_at",
                      "thumb_version", "cover_hash"):
            if field in album and not isinstance(album[field], str):
                raise ValueError(f"album {field} must be text")
        if "description_truncated" in album and not isinstance(album["description_truncated"], bool):
            raise ValueError("album description_truncated must be a boolean")
        try:
            if "price" in album and (type(album["price"]) not in (int, float)
                                     or not math.isfinite(album["price"]) or album["price"] < 0):
                raise ValueError("invalid price")
        except OverflowError:
            raise ValueError("invalid price") from None
        if album.get("thumb") and not re.fullmatch(r"thumbs/[a-z0-9_-]+/[0-9]+\.(?:jpg|png|webp)", album["thumb"]):
            raise ValueError("unsafe cover path")
        links = album.get("links")
        if not isinstance(links, list):
            raise ValueError("purchase links must be an array")
        if any(not isinstance(link, dict) or not isinstance(link.get("url"), str)
               or not isinstance(link.get("platform"), str) for link in links):
            raise ValueError("invalid purchase link")
        for url in [album["album_url"], *(link["url"] for link in links)]:
            parsed = urlsplit(url)
            if parsed.scheme != "https" or not parsed.hostname or parsed.username or parsed.password:
                raise ValueError("album links must be public HTTPS URLs without credentials")
        evidence = album.get("price_evidence")
        if isinstance(evidence, dict) and "candidates" in evidence and not isinstance(evidence["candidates"], list):
            raise ValueError("price candidates must be an array")
        if "history" in album and (not isinstance(album["history"], list) or
                                  any(not isinstance(h, dict) or not isinstance(h.get("at"), str)
                                      for h in album["history"])):
            raise ValueError("invalid change history")
    return value
