"""Catalog storage: per-seller shards plus an index manifest, all atomic writes."""

from __future__ import annotations

import json
import os
import time
from pathlib import Path


def atomic_write_json(path: Path, obj: object) -> None:
    """Write JSON via a temp file + os.replace so readers never see a partial file.

    If serialization fails, the original file (if any) is left untouched - the
    temp file never gets created before json.dumps succeeds. On Windows the
    rename step can lose against a reader holding the destination open (the
    viewer serving the very shard being rewritten) or against antivirus and
    indexer locks; those clear in moments, so the replace is retried a few
    times before giving up. Any OSError qualifies: a sharing violation
    arrives as PermissionError (WinError 5 or 32) but a rename can also fail
    transiently for other OS reasons, and none of them should leave the temp
    file behind.
    """
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_name(path.name + ".tmp")
    # Compact separators: shards are machine-read by the viewer, and the
    # indent=1 form measured ~35% larger on real catalog data.
    tmp.write_text(
        json.dumps(obj, ensure_ascii=False, separators=(",", ":")), encoding="utf-8"
    )
    for attempt in range(5):
        try:
            os.replace(tmp, path)
            return
        except OSError:
            if attempt == 4:
                # The original is untouched; the orphaned temp would
                # otherwise linger until the next write overwrites it.
                tmp.unlink(missing_ok=True)
                raise
            time.sleep(0.1 * 2 ** attempt)


def _shard_path(data_dir: Path, seller: str) -> Path:
    return data_dir / "catalog" / f"{seller}.json"


def load_shard(data_dir: Path, seller: str) -> dict[str, dict]:
    """Load a seller's shard, keyed by album id. Missing shard - empty dict.

    A corrupt shard raises ValueError naming the file: the scraper must not
    half-merge against unreadable data, and the CLI turns it into a seller
    error rather than crashing every other seller's run with it.
    """
    path = _shard_path(data_dir, seller)
    if not path.exists():
        return {}
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
        albums = payload["albums"]
        return {album["id"]: album for album in albums}
    except (ValueError, KeyError, TypeError) as exc:
        raise ValueError(f"corrupt shard {path}: {exc}") from exc


def write_shard(data_dir: Path, seller: str, albums: dict[str, dict]) -> None:
    """Write a seller's shard, sorted by id descending (newest Yupoo albums first)."""
    ordered = sorted(albums.values(), key=lambda a: int(a["id"]), reverse=True)
    atomic_write_json(_shard_path(data_dir, seller), {"seller": seller, "albums": ordered})


# Facts about the album itself, not about this scrape run: once captured they
# stay captured. A later fetch that fails to extract them (layout variant,
# truncated JSON-LD) must not erase what an earlier run already learned.
_STICKY_FIELDS = ("date_published", "date_modified")


def merge_album(old: dict | None, new: dict, run_ts: str) -> dict:
    """Merge a freshly-scraped record with the prior one, keeping first_seen stable.

    A record seen for the first time gets first_seen = run_ts. A previously known
    record keeps its original first_seen no matter what changed. updated_at is
    always stamped with the current run_ts. Sticky fields fall back to the old
    value when the fresh scrape came up empty.
    """
    merged = dict(new)
    merged["first_seen"] = old["first_seen"] if old else run_ts
    merged["updated_at"] = run_ts
    if old:
        for field in _STICKY_FIELDS:
            if not merged.get(field) and old.get(field):
                merged[field] = old[field]
    return merged


def write_index(data_dir: Path, sellers: list[dict], shops: list[dict], run_ts: str) -> None:
    """Write the top-level index manifest the viewer reads to find every shard."""
    atomic_write_json(
        data_dir / "index.json",
        {"generated_at": run_ts, "sellers": sellers, "shops": shops},
    )
