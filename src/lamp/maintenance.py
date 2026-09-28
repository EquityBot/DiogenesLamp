"""Read-only diagnosis and explicit local metadata recovery."""

from __future__ import annotations

import json
import uuid
import zipfile
from datetime import UTC, datetime
from pathlib import Path

from lamp.covers import cover_facts, resize_cover
from lamp.locking import catalogue_lock
from lamp.schema import data_path, validate_metadata
from lamp.storage import atomic_write_json

MAX_METADATA_BYTES = 256 * 1024 * 1024
MAX_METADATA_FILES = 10000


def read_metadata(path: Path, relative: str) -> dict:
    if path.stat().st_size > MAX_METADATA_BYTES:
        raise ValueError("metadata file exceeds 256 MB")
    return validate_metadata(json.loads(path.read_text(encoding="utf-8")), relative)


def metadata_files(root: Path) -> list[Path]:
    files = [*([root / "index.json"] if (root / "index.json").exists() else []),
             *sorted((root / "catalog").glob("*.json")),
             *sorted((root / "runs").glob("*.json"))]
    if len(files) > MAX_METADATA_FILES:
        raise ValueError("too many metadata files")
    for file in files:
        data_path(root, file.relative_to(root).as_posix())
    if sum(file.stat().st_size for file in files) > MAX_METADATA_BYTES:
        raise ValueError("metadata exceeds 256 MB")
    return files


def diagnose(root: Path) -> dict:
    issues: list[str] = []
    report: dict = {"schema_version": 1, "albums": 0, "missing_covers": 0, "unpriced": 0,
              "without_detail_date": 0, "without_description": 0, "gone": 0,
              "detail_failures": 0, "issues": issues}
    try:
        index = read_metadata(data_path(root, "index.json"), "index.json")
    except (OSError, ValueError) as exc:
        issues.append(f"index.json: {exc}")
        return report
    ids = set()
    for seller in index["sellers"]:
        relative = seller["shard"]
        try:
            shard = read_metadata(data_path(root, relative), relative)
            if "album_count" in seller and seller["album_count"] != len(shard["albums"]):
                issues.append(f"{relative}: index count differs from shard")
            for album in shard["albums"]:
                report["albums"] += 1
                if album["id"] in ids:
                    issues.append(f"{relative}: duplicate album ID across shards: {album['id']}")
                ids.add(album["id"])
                report["unpriced"] += int(not album.get("price"))
                report["gone"] += int(bool(album.get("gone_since")))
                report["without_detail_date"] += int(not album.get("updated_at"))
                # Absent, not empty: records written before descriptions were kept.
                report["without_description"] += int("description" not in album)
                if album.get("thumb") and not data_path(root, album["thumb"], "cover").is_file():
                    report["missing_covers"] += 1
            run_relative = f"runs/{seller['id']}.json"
            run_path = data_path(root, run_relative)
            if run_path.exists():
                report["detail_failures"] += len(read_metadata(run_path, run_relative).get("failed_ids", []))
        except (OSError, ValueError) as exc:
            issues.append(f"{relative}: {exc}")
    return report


def _snapshot(root: Path, validate: bool = True) -> Path:
    files = metadata_files(root)
    # Validate every source before creating an archive that claims to be restorable.
    if validate:
        index = read_metadata(data_path(root, "index.json"), "index.json")
        for seller in index["sellers"]:
            if not data_path(root, seller["shard"]).is_file():
                raise ValueError(f"{seller['shard']}: missing indexed shard")
        for file in files:
            read_metadata(file, file.relative_to(root).as_posix())
    directory = root / "snapshots"
    directory.mkdir(exist_ok=True)
    if not directory.resolve().is_relative_to(root.resolve()):
        raise ValueError("snapshot directory escapes data directory")
    path = directory / (datetime.now(UTC).strftime("%Y%m%dT%H%M%S") + "-" + uuid.uuid4().hex[:8] + ".zip")
    try:
        with zipfile.ZipFile(path, "x", compression=zipfile.ZIP_DEFLATED) as archive:
            for file in files:
                archive.write(file, file.relative_to(root).as_posix())
    except BaseException:
        path.unlink(missing_ok=True)
        raise
    return path


def snapshot(root: Path) -> Path:
    with catalogue_lock(root):
        return _snapshot(root)


def restore(root: Path, archive_path: Path) -> Path | None:
    with catalogue_lock(root):
        with zipfile.ZipFile(archive_path) as archive:
            infos = archive.infolist()
            if len(infos) > MAX_METADATA_FILES or sum(i.file_size for i in infos) > MAX_METADATA_BYTES:
                raise ValueError("snapshot exceeds recovery limits")
            records = {}
            for info in infos:
                data_path(root, info.filename)
                if info.filename in records:
                    raise ValueError("duplicate snapshot member")
                records[info.filename] = validate_metadata(json.loads(archive.read(info)), info.filename)
            if "index.json" not in records:
                raise ValueError("snapshot has no index")
            if any(s["shard"] not in records for s in records["index.json"]["sellers"]):
                raise ValueError("snapshot is missing an indexed shard")
        previous = _snapshot(root, validate=False) if metadata_files(root) else None
        # Shards precede the manifest. Individual replacements are atomic;
        # the previous snapshot provides recovery if the process is interrupted.
        for relative, payload in records.items():
            if relative != "index.json":
                atomic_write_json(data_path(root, relative), payload)
        atomic_write_json(data_path(root, "index.json"), records["index.json"])
        return previous


def prepare_covers(root: Path, limit: int) -> dict:
    result: dict = {"processed": 0, "resized": 0, "before_bytes": 0, "after_bytes": 0, "errors": []}
    with catalogue_lock(root):
        result["snapshot"] = str(_snapshot(root))
        index = read_metadata(data_path(root, "index.json"), "index.json")
        for seller in index["sellers"]:
            if result["processed"] >= limit:
                break
            relative = seller["shard"]
            shard = read_metadata(data_path(root, relative), relative)
            changed = False
            for album in shard["albums"]:
                if result["processed"] >= limit:
                    break
                if not album.get("thumb"):
                    continue
                path = data_path(root, album["thumb"], "cover")
                # Completed covers are skipped, making bounded runs resumable.
                if album.get("cover_prepared") == 1 and path.is_file():
                    continue
                result["processed"] += 1
                try:
                    if path.stat().st_size > 8 * 1024 * 1024:
                        raise ValueError("cover exceeds 8 MB")
                    result["before_bytes"] += path.stat().st_size
                    result["resized"] += int(resize_cover(path))
                    result["after_bytes"] += path.stat().st_size
                    album.update(cover_facts(path), cover_prepared=1)
                    changed = True
                except (OSError, ValueError) as exc:
                    result["errors"].append(f"{album['id']}: {exc}")
            if changed:
                shard["schema_version"] = 1
                atomic_write_json(data_path(root, relative), shard)
    return result
