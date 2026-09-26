"""Small validation boundary for local catalogue and recovery files."""

from __future__ import annotations

import math
import re
from pathlib import Path
from urllib.parse import urlsplit


def data_path(root: Path, relative: str, kind: str = "metadata") -> Path:
    pattern = (r"thumbs/[a-z0-9_-]+/[0-9]+\.(?:jpg|png|webp)" if kind == "cover" else
               r"(?:index\.json|(?:catalog|runs)/[a-z0-9_-]+\.json)")
    if not isinstance(relative, str) or not re.fullmatch(pattern, relative):
        raise ValueError("unsafe catalogue path")
    path = (root / relative).resolve()
    if not path.is_relative_to(root.resolve()):
        raise ValueError("catalogue path escapes data directory")
    return path


def validate_metadata(value: object, relative: str) -> dict:
    if (not isinstance(value, dict) or type(value.get("schema_version", 1)) is not int
            or value.get("schema_version", 1) != 1):
        raise ValueError("unsupported metadata shape or schema version")
    if relative == "index.json":
        sellers = value.get("sellers")
        if not isinstance(sellers, list):
            raise ValueError("index sellers must be an array")
        seen = set()
        for seller in sellers:
            if (not isinstance(seller, dict) or not isinstance(seller.get("id"), str)
                    or not re.fullmatch(r"[a-z0-9_-]+", seller["id"])):
                raise ValueError("invalid seller ID")
            if seller["id"] in seen or seller.get("shard") != f"catalog/{seller['id']}.json":
                raise ValueError("duplicate seller or mismatched shard path")
            seen.add(seller["id"])
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
        for field in ("category", "first_seen", "updated_at",
                      "thumb_version", "cover_hash"):
            if field in album and not isinstance(album[field], str):
                raise ValueError(f"album {field} must be text")
        if "price" in album and (type(album["price"]) not in (int, float)
                                 or not math.isfinite(album["price"]) or album["price"] < 0):
            raise ValueError("invalid price")
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
