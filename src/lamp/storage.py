"""Catalogue storage: per-seller shards plus an index manifest, all atomic writes."""

from __future__ import annotations

import json
import os
import tempfile
import time
from pathlib import Path

from lamp.schema import data_path, validate_metadata


def atomic_write_json(path: Path, obj: object) -> None:
    """Write JSON via a temp file + os.replace so readers never see a partial file.

    If serialisation fails, the original file (if any) is left untouched - the
    temp file never gets created before json.dumps succeeds. On Windows the
    rename step can lose against a reader holding the destination open (the
    viewer serving the very shard being rewritten) or against antivirus and
    indexer locks; those clear in moments, so the replace is retried a few
    times before giving up. Any OSError qualifies: a sharing violation
    arrives as PermissionError (WinError 5 or 32) but a rename can also fail
    transiently for other OS reasons, and none of them should leave the temp
    file behind.
    """
    body = json.dumps(obj, ensure_ascii=False, separators=(",", ":"))
    path.parent.mkdir(parents=True, exist_ok=True)
    # Compact separators: shards are machine-read by the viewer, and the
    # indent=1 form measured ~35% larger on real catalogue data.
    with tempfile.NamedTemporaryFile(mode="w", encoding="utf-8", dir=path.parent,
                                     prefix=path.name + ".", suffix=".tmp", delete=False) as file:
        tmp = Path(file.name)
        try:
            file.write(body)
        except BaseException:
            file.close()
            tmp.unlink(missing_ok=True)
            raise
    try:
        for attempt in range(5):
            try:
                os.replace(tmp, path)
                return
            except OSError:
                if attempt == 4:
                    raise
                time.sleep(0.1 * 2**attempt)
    finally:
        tmp.unlink(missing_ok=True)


def _shard_path(data_dir: Path, seller: str) -> Path:
    return data_path(data_dir, f"catalog/{seller}.json")


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
        payload = validate_metadata(json.loads(path.read_text(encoding="utf-8")),
                                    f"catalog/{seller}.json")
        albums = payload["albums"]
        return {album["id"]: album for album in albums}
    except (ValueError, KeyError, TypeError) as exc:
        raise ValueError(f"corrupt shard {path}: {exc}") from exc


def write_shard(data_dir: Path, seller: str, albums: dict[str, dict]) -> None:
    """Write a seller's shard, sorted by id descending (newest Yupoo albums first)."""
    ordered = sorted(albums.values(), key=lambda a: int(a["id"]), reverse=True)
    atomic_write_json(_shard_path(data_dir, seller),
                      {"schema_version": 1, "seller": seller, "albums": ordered})


# Facts about the album itself, not about this scrape run: once captured they
# stay captured. A later fetch that fails to extract them (layout variant,
# truncated JSON-LD) must not erase what an earlier run already learnt.
# Price is not sticky: an absent price is a catalogue change, not a missed
# extract of a date-like field.
_STICKY_FIELDS = ("date_published", "date_modified")


def record_change(album: dict, field: str, before: object, after: object, run_ts: str) -> None:
    if before == after:
        return
    history = list(album.get("history", []))
    history.append({"at": run_ts, "field": field, "before": before, "after": after})
    album["history"] = history[-50:]


def merge_album(old: dict | None, new: dict, run_ts: str) -> dict:
    """Merge a freshly-scraped record with the prior one, keeping first_seen stable.

    A record seen for the first time gets first_seen = run_ts. A previously known
    record keeps its original first_seen no matter what changed. updated_at is
    always stamped with the current run_ts. Sticky fields fall back to the old
    value when the fresh scrape came up empty.
    """
    merged = dict(new)
    if old:
        merged["history"] = list(old.get("history", []))
        if old.get("gone_since"):
            merged["gone_since"] = old["gone_since"]
        for key in ("thumb_version", "cover_hash", "cover_prepared"):
            if key not in merged and key in old:
                merged[key] = old[key]
        if old.get("price") and new.get("price"):
            record_change(merged, "price", old["price"], new["price"], run_ts)
        elif bool(old.get("price")) != bool(new.get("price")):
            record_change(merged, "price availability", "known" if old.get("price") else "unknown",
                          "known" if new.get("price") else "unknown", run_ts)
        record_change(merged, "purchase links", old.get("links", []), new.get("links", []), run_ts)
    merged["first_seen"] = old.get("first_seen", run_ts) if old else run_ts
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
        {"schema_version": 1, "generated_at": run_ts, "sellers": sellers, "shops": shops},
    )


def read_run(data_dir: Path, seller: str) -> dict:
    relative = f"runs/{seller}.json"
    path = data_path(data_dir, relative)
    try:
        return validate_metadata(json.loads(path.read_text(encoding="utf-8")), relative)
    except FileNotFoundError:
        return {}
    except (OSError, ValueError) as exc:
        raise ValueError(f"unreadable run summary {path}: {exc}") from exc


def write_run(data_dir: Path, seller: str, run_ts: str, summary: dict) -> None:
    previous = read_run(data_dir, seller)
    summary["schema_version"] = 1
    resolved = set(summary.pop("resolved_ids", []))
    failed = (set(previous.get("failed_ids", [])) - resolved) | set(summary.get("failed_ids", []))
    summary["failed_ids"] = sorted(failed)
    summary["last_attempt"] = run_ts
    summary["last_success"] = previous.get("last_success", "")
    summary["last_listing"] = (run_ts if summary.get("listing_complete") else
                               previous.get("last_listing", ""))
    if summary.get("listing_complete") and not summary.get("error") and not failed:
        summary["last_success"] = run_ts
    atomic_write_json(data_path(data_dir, f"runs/{seller}.json"), summary)
