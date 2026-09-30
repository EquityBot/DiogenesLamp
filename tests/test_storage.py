import io
import json
import ntpath
import os
import threading
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import pytest

from lamp.schema import data_path
from lamp.storage import (
    atomic_write_json,
    load_shard,
    merge_album,
    read_run,
    write_index,
    write_run,
    write_shard,
)


def test_corrupt_run_status_is_reported_and_preserved(tmp_path):
    path = tmp_path / "runs" / "sellerx.json"
    path.parent.mkdir()
    path.write_text('{"failed_ids":"101"}', encoding="utf-8")
    with pytest.raises(ValueError, match="unreadable run summary"):
        read_run(tmp_path, "sellerx")
    with pytest.raises(ValueError, match="unreadable run summary"):
        write_run(tmp_path, "sellerx", "2026-09-26", {})
    assert path.read_text(encoding="utf-8") == '{"failed_ids":"101"}'

RECORD = {
    "id": "111",
    "seller": "sellerx",
    "title": "Item A",
    "list_title": "Item A",
    "thumb": "thumbs/sellerx/111.jpg",
    "album_url": "https://sellerx.x.yupoo.com/albums/111",
    "links": [{"platform": "weidian", "url": "https://weidian.com/item.html?itemID=9"}],
    "category": "hoodies",
    "photo_count": 23,
    "first_seen": "2026-08-01T00:00:00+10:00",
    "updated_at": "2026-08-01T00:00:00+10:00",
}


def test_merge_album_new_gets_run_ts():
    fresh = dict(RECORD, first_seen="", updated_at="")
    merged = merge_album(None, fresh, "2026-08-17T12:00:00+10:00")
    assert merged["first_seen"] == "2026-08-17T12:00:00+10:00"
    assert merged["updated_at"] == "2026-08-17T12:00:00+10:00"


def test_merge_legacy_album_without_first_seen():
    old = {key: value for key, value in RECORD.items() if key != 'first_seen'}
    assert merge_album(old, RECORD, 'now')['first_seen'] == 'now'


def test_merge_album_existing_keeps_first_seen():
    fresh = dict(RECORD, title="Item A v2", first_seen="", updated_at="")
    merged = merge_album(RECORD, fresh, "2026-08-17T12:00:00+10:00")
    assert merged["first_seen"] == "2026-08-01T00:00:00+10:00"
    assert merged["updated_at"] == "2026-08-17T12:00:00+10:00"
    assert merged["title"] == "Item A v2"


def test_merge_album_keeps_known_dates_when_rescrape_finds_none():
    """A layout variant that yields no JSON-LD must not erase a captured date."""
    known = dict(RECORD, date_published="2025-11-24", date_modified="2026-08-16")
    fresh = dict(RECORD, date_published="", date_modified="", first_seen="", updated_at="")
    merged = merge_album(known, fresh, "2026-08-17T12:00:00+10:00")
    assert merged["date_published"] == "2025-11-24"
    assert merged["date_modified"] == "2026-08-16"


def test_merge_album_fresh_dates_win_over_old():
    known = dict(RECORD, date_published="2025-11-24", date_modified="2025-11-24")
    fresh = dict(RECORD, date_published="2025-11-24", date_modified="2026-08-17",
                 first_seen="", updated_at="")
    merged = merge_album(known, fresh, "2026-08-17T12:00:00+10:00")
    assert merged["date_modified"] == "2026-08-17"


def test_merge_album_absent_price_does_not_keep_the_old_price():
    """A listing that no longer carries a price must not keep the old one."""
    known = dict(RECORD, price=180)
    fresh = dict(RECORD, price=0, first_seen="", updated_at="")
    merged = merge_album(known, fresh, "2026-08-17T12:00:00+10:00")
    assert merged["price"] == 0
    assert all(event["field"] != "price" for event in merged["history"])


def test_change_history_is_bounded_and_unchanged_scrapes_add_nothing():
    old = dict(RECORD, price=180)
    for amount in range(181, 241):
        old = merge_album(old, dict(RECORD, price=amount), f"run-{amount}")
    assert len(old["history"]) == 50
    same = merge_album(old, dict(RECORD, price=240), "later")
    assert same["history"] == old["history"]


def test_shard_roundtrip_sorted(tmp_path: Path):
    albums = {"5": dict(RECORD, id="5"), "10": dict(RECORD, id="10")}
    write_shard(tmp_path, "sellerx", albums)
    loaded = load_shard(tmp_path, "sellerx")
    assert list(loaded) == ["10", "5"]  # descending id order
    assert loaded == {"10": dict(RECORD, id="10"), "5": dict(RECORD, id="5")}


def test_load_missing_shard_empty(tmp_path: Path):
    assert load_shard(tmp_path, "ghost") == {}


def test_corrupt_shard_raises_with_filename(tmp_path: Path):
    """A shard that will not parse must name the file, not crash the whole
    run with a bare JSONDecodeError, and must not be silently dropped."""
    catalog = tmp_path / "catalog"
    catalog.mkdir()
    (catalog / "bad.json").write_text("{broken", encoding="utf-8")
    with pytest.raises(ValueError, match="corrupt shard.*bad.json"):
        load_shard(tmp_path, "bad")
    # structurally wrong but valid JSON is also corrupt
    (catalog / "noid.json").write_text('{"albums": [{"title": "x"}]}', encoding="utf-8")
    with pytest.raises(ValueError, match="corrupt shard"):
        load_shard(tmp_path, "noid")


def test_atomic_write_leaves_no_tmp(tmp_path: Path):
    target = tmp_path / "x.json"
    atomic_write_json(target, {"ok": 1})
    assert json.loads(target.read_text(encoding="utf-8")) == {"ok": 1}
    assert list(tmp_path.iterdir()) == [target]


def test_atomic_write_failure_preserves_original(tmp_path: Path):
    target = tmp_path / "x.json"
    atomic_write_json(target, {"ok": 1})
    with pytest.raises(TypeError):
        atomic_write_json(target, {"bad": {1, 2}})  # sets are not JSON-serialisable
    assert json.loads(target.read_text(encoding="utf-8")) == {"ok": 1}
    assert list(tmp_path.iterdir()) == [target]


def test_atomic_write_close_failure_removes_temporary_file(tmp_path, monkeypatch):
    from lamp import storage

    target = tmp_path / "x.json"
    target.write_text('{"old":true}', encoding="utf-8")
    failure = OSError(28, "injected buffered write failure")
    real_temporary = storage.tempfile.NamedTemporaryFile
    streams = []

    class FailingRaw(io.RawIOBase):
        def __init__(self, name):
            self.name = name

        def writable(self):
            return True

        def write(self, _data):
            raise failure

    def temporary(**kwargs):
        with real_temporary(**kwargs) as original:
            name = original.name
        stream = io.TextIOWrapper(io.BufferedWriter(FailingRaw(name)), encoding="utf-8")
        streams.append(stream)
        return stream

    def unexpected_replace(*_args):
        pytest.fail("replacement must not run after a staging failure")

    monkeypatch.setattr(storage.tempfile, "NamedTemporaryFile", temporary)
    monkeypatch.setattr(storage.os, "replace", unexpected_replace)
    with pytest.raises(OSError) as result:
        atomic_write_json(target, {"new": True})
    assert result.value is failure
    assert target.read_text(encoding="utf-8") == '{"old":true}'
    assert streams[0].closed
    assert list(tmp_path.iterdir()) == [target]


def test_atomic_write_encoding_failure_preserves_original(tmp_path):
    target = tmp_path / "x.json"
    target.write_text('{"old":true}', encoding="utf-8")
    with pytest.raises(UnicodeEncodeError):
        atomic_write_json(target, {"bad": "\ud800"})
    assert target.read_text(encoding="utf-8") == '{"old":true}'
    assert list(tmp_path.iterdir()) == [target]


def test_atomic_write_retries_a_transient_windows_lock(tmp_path: Path, monkeypatch):
    """Windows denies a rename over a destination a reader holds open (the
    viewer serving the shard being re-written) or while AV/indexer holds a
    lock. Those clear in moments, so the replace retries; a permanent
    failure still raises, and any OSError counts - a sharing violation is
    not always spelt PermissionError."""
    target = tmp_path / "x.json"
    atomic_write_json(target, {"ok": 1})

    from lamp import storage

    real_replace = storage.os.replace
    calls = 0

    def flaky_replace(src, dst):
        nonlocal calls
        calls += 1
        if calls < 3:
            raise PermissionError("WinError 32 sharing violation")
        real_replace(src, dst)

    monkeypatch.setattr(storage.os, "replace", flaky_replace)
    monkeypatch.setattr(storage.time, "sleep", lambda _s: None)  # no real delay
    atomic_write_json(target, {"ok": 2})
    assert calls == 3
    assert json.loads(target.read_text(encoding="utf-8")) == {"ok": 2}

    def always_locked(src, dst):
        raise OSError("rename refused for some other OS reason")

    monkeypatch.setattr(storage.os, "replace", always_locked)
    with pytest.raises(OSError):
        atomic_write_json(target, {"ok": 3})
    assert json.loads(target.read_text(encoding="utf-8")) == {"ok": 2}
    assert list(tmp_path.iterdir()) == [target]  # no orphaned .tmp


def test_write_index(tmp_path: Path):
    sellers = [{"id": "sellerx", "name": "sellerx", "url": "https://sellerx.x.yupoo.com",
                "album_count": 2, "shard": "catalog/sellerx.json"}]
    shops = [{"id": "weidian-7", "platform": "weidian", "name": "weidian 7",
              "url": "https://weidian.com/?userid=7"}]
    write_index(tmp_path, sellers, shops, "2026-08-17T12:00:00+10:00")
    index = json.loads((tmp_path / "index.json").read_text(encoding="utf-8"))
    assert index["generated_at"] == "2026-08-17T12:00:00+10:00"
    assert index["sellers"] == sellers
    assert index["shops"] == shops


def test_concurrent_atomic_writes_use_independent_temporary_files(tmp_path, monkeypatch):
    from lamp import storage

    barrier = threading.Barrier(2)
    original_replace = storage.os.replace
    seen = []

    def replace(src, dst):
        if src not in seen:
            seen.append(src)
            barrier.wait(timeout=5)
        original_replace(src, dst)

    monkeypatch.setattr(storage.os, "replace", replace)
    target = tmp_path / "shared.json"
    with ThreadPoolExecutor(max_workers=2) as workers:
        futures = [workers.submit(atomic_write_json, target, {"writer": name})
                   for name in ("A", "B")]
        for future in futures:
            future.result(timeout=10)
    assert len(set(seen)) == 2
    assert json.loads(target.read_text())["writer"] in ("A", "B")
    assert list(tmp_path.iterdir()) == [target]


def test_data_path_survives_a_sibling_creating_its_directory_mid_call(tmp_path, monkeypatch):
    """Two scrape_all seller threads raced: one resolved catalog/<seller>.json
    while catalog/ was missing and the other created it mid-call. Windows
    realpath then kept the extended-length prefix on the result, which failed
    the containment check. The mkdir is replayed inside the first failing
    lookup so the interleaving is exact and needs no threads."""
    lookup = getattr(ntpath, "_getfinalpathname", None)
    if lookup is None:
        pytest.skip("replays a Windows realpath internal")

    def racing(path):
        try:
            return lookup(path)
        except OSError:
            (tmp_path / "catalog").mkdir(exist_ok=True)
            raise

    monkeypatch.setattr(ntpath, "_getfinalpathname", racing)
    expected = tmp_path.resolve() / "catalog" / "sellery.json"
    assert data_path(tmp_path, "catalog/sellery.json") == expected


def test_data_path_rejects_a_catalogue_directory_linked_outside_the_data_directory(tmp_path):
    """A catalog/ link that leaves the data directory fails containment, as it
    did when the whole target was resolved."""
    root = tmp_path / "data"
    root.mkdir()
    outside = tmp_path / "outside"
    outside.mkdir()
    try:
        os.symlink(outside, root / "catalog", target_is_directory=True)
    except OSError:
        pytest.skip("creating symlinks needs a privilege this account lacks")
    with pytest.raises(ValueError, match="escapes data directory"):
        data_path(root, "catalog/sellerx.json")


def test_data_path_works_before_the_data_directory_exists(tmp_path):
    root = tmp_path / "missing"
    assert data_path(root, "index.json") == root.resolve() / "index.json"
