import json
import math
import sys
import zipfile
from pathlib import Path

import pytest
from PIL import Image, ImageDraw, ImageEnhance, ImageOps

from lamp.cli import main
from lamp.covers import cover_facts, resize_cover
from lamp.maintenance import diagnose, prepare_covers, restore, snapshot
from lamp.schema import data_path, validate_metadata
from lamp.storage import atomic_write_json, load_shard


def catalogue(root):
    atomic_write_json(root / "index.json", {"schema_version": 1, "sellers": [
        {"id": "example", "shard": "catalog/example.json", "album_count": 1}]})
    atomic_write_json(root / "catalog/example.json", {"seller": "example", "albums": [
        {"id": "123", "seller": "example", "title": "Example", "price": 0,
         "album_url": "https://example.x.yupoo.com/albums/123", "links": [],
         "thumb": "thumbs/example/123.jpg"}]})


def test_doctor_is_read_only_and_reports_coverage(tmp_path):
    catalogue(tmp_path)
    before = {p.relative_to(tmp_path): p.read_bytes() for p in tmp_path.rglob("*") if p.is_file()}
    result = diagnose(tmp_path)
    assert result["issues"] == []
    assert result["albums"] == result["missing_covers"] == result["unpriced"] == 1
    assert before == {p.relative_to(tmp_path): p.read_bytes() for p in tmp_path.rglob("*") if p.is_file()}
    assert main(["--data", str(tmp_path), "doctor"]) == 0


def test_snapshot_restores_corrupt_metadata_and_keeps_previous_bytes(tmp_path):
    catalogue(tmp_path)
    archive = snapshot(tmp_path)
    shard = tmp_path / "catalog/example.json"
    original = shard.read_bytes()
    shard.write_text("corrupt", encoding="utf-8")
    previous = restore(tmp_path, archive)
    assert shard.read_bytes() == original
    with zipfile.ZipFile(previous) as saved:
        assert saved.read("catalog/example.json") == b"corrupt"
    assert diagnose(tmp_path)["issues"] == []


def test_snapshot_rejects_missing_indexed_shard_before_creating_archive(tmp_path):
    catalogue(tmp_path)
    (tmp_path / "catalog/example.json").unlink()
    with pytest.raises(ValueError, match="catalog/example.json: missing indexed shard"):
        snapshot(tmp_path)
    assert not (tmp_path / "snapshots").exists()


@pytest.mark.parametrize("name,payload", [
    ("../outside.json", {}), ("catalog/../../outside.json", {}),
    ("catalog/C:bad.json", {}), ("index.json", {"schema_version": 99}),
    ("index.json", {"sellers": [{"id": "example", "shard": "catalog/example.json"}]}),
])
def test_unsafe_or_incomplete_snapshot_never_changes_metadata(tmp_path, name, payload):
    catalogue(tmp_path)
    before = (tmp_path / "index.json").read_bytes()
    bad = tmp_path / "bad.zip"
    with zipfile.ZipFile(bad, "w") as archive:
        archive.writestr(name, json.dumps(payload))
    with pytest.raises(ValueError):
        restore(tmp_path, bad)
    assert (tmp_path / "index.json").read_bytes() == before
    assert not (tmp_path / "snapshots").exists()


@pytest.mark.parametrize("target,field,value", [
    ("shard", "seller", "other"), ("shard", "seller", None),
    ("album", "seller", "other"), ("album", "seller", None),
    ("album", "title", None), ("album", "album_url", None),
    ("album", "thumb", None), ("album", "links", None),
    ("album", "links", [{"url": "https://example.com/item"}]),
    ("album", "links", [{"platform": "shop", "url": ""}]),
    ("album", "price_evidence", {"candidates": {}}),
    ("shard", "schema_version", True),
])
def test_restore_rejects_metadata_the_viewer_cannot_use(tmp_path, target, field, value):
    catalogue(tmp_path)
    files = [tmp_path / "index.json", tmp_path / "catalog/example.json"]
    before = {path: path.read_bytes() for path in files}
    shard = json.loads(before[files[1]])
    record = shard if target == "shard" else shard["albums"][0]
    if value is None:
        record.pop(field)
    else:
        record[field] = value
    bad = tmp_path / "bad.zip"
    with zipfile.ZipFile(bad, "w") as archive:
        archive.writestr("index.json", before[files[0]])
        archive.writestr("catalog/example.json", json.dumps(shard))
    with pytest.raises(ValueError):
        restore(tmp_path, bad)
    assert before == {path: path.read_bytes() for path in files}
    assert not (tmp_path / "snapshots").exists()


def test_schema_rejects_duplicate_ids_unsafe_covers_and_links(tmp_path):
    catalogue(tmp_path)
    original = json.loads((tmp_path / "catalog/example.json").read_text())["albums"][0]
    for changes in [{"thumb": "../outside.jpg"}, {"price": float("nan")},
                    {"links": [{"platform": "shop", "url": "javascript:alert(1)"}]}]:
        with pytest.raises(ValueError):
            validate_metadata({"seller": "example", "albums": [original | changes]},
                              "catalog/example.json")
    with pytest.raises(ValueError):
        validate_metadata({"seller": "example", "albums": [original, original]},
                          "catalog/example.json")
    with pytest.raises(ValueError):
        data_path(tmp_path, "runs/../index.json")


def test_cover_conversion_is_bounded_oriented_and_resumable(tmp_path):
    catalogue(tmp_path)
    cover = tmp_path / "thumbs/example/123.jpg"
    cover.parent.mkdir(parents=True)
    image = Image.new("RGB", (1600, 800), "red")
    exif = Image.Exif()
    exif[274] = 6
    image.save(cover, exif=exif)
    before = cover.stat().st_size
    result = prepare_covers(tmp_path, 1)
    assert result["resized"] == result["processed"] == 1
    assert result["after_bytes"] < before
    with Image.open(cover) as resized:
        assert resized.size == (320, 640)
        assert resized.getexif().get(274, 1) == 1
    assert prepare_covers(tmp_path, 1)["processed"] == 0
    facts = cover_facts(cover)
    assert facts == cover_facts(cover)
    assert len(facts["thumb_version"]) == 24
    assert json.loads((tmp_path / "catalog/example.json").read_text())["albums"][0]["thumb_version"] == facts["thumb_version"]


def test_transparency_survives_and_failed_conversion_preserves_original(tmp_path):
    png = tmp_path / "alpha.png"
    Image.new("RGBA", (1000, 800), (20, 40, 60, 80)).save(png)
    resize_cover(png)
    with Image.open(png) as converted:
        assert converted.mode == "RGBA"
        assert converted.getpixel((0, 0))[3] == 80
    png.write_bytes(b"not an image")
    with pytest.raises(OSError):
        resize_cover(png)
    assert png.read_bytes() == b"not an image"


def test_restore_preserves_orphaned_shards_when_index_is_missing(tmp_path):
    catalogue(tmp_path)
    archive = snapshot(tmp_path)
    (tmp_path / "index.json").unlink()
    (tmp_path / "catalog/example.json").write_text("orphaned bytes", encoding="utf-8")
    previous = restore(tmp_path, archive)
    with zipfile.ZipFile(previous) as saved:
        assert saved.read("catalog/example.json") == b"orphaned bytes"


def test_cover_fingerprints_tolerate_brightness_but_distinguish_geometry(tmp_path):
    image = Image.new("RGB", (256, 256), "white")
    draw = ImageDraw.Draw(image)
    draw.rectangle((12, 22, 83, 178), fill="black")
    draw.ellipse((107, 73, 236, 221), fill="grey")
    paths = [tmp_path / f"sample{i}.png" for i in range(3)]
    image.save(paths[0])
    ImageEnhance.Brightness(image).enhance(.85).save(paths[1])
    ImageOps.mirror(image).save(paths[2])
    hashes = [int(cover_facts(path)["cover_hash"], 16) for path in paths]
    assert (hashes[0] ^ hashes[1]).bit_count() <= 8
    assert (hashes[0] ^ hashes[2]).bit_count() > 8


def test_decompression_bomb_does_not_abort_local_preparation(tmp_path, monkeypatch):
    catalogue(tmp_path)
    cover = tmp_path / "thumbs/example/123.jpg"
    cover.parent.mkdir(parents=True)
    Image.new("RGB", (100, 100)).save(cover)
    monkeypatch.setattr(Image, "MAX_IMAGE_PIXELS", 10)
    result = prepare_covers(tmp_path, 1)
    assert result["resized"] == 0
    assert "cover dimensions exceed limits" in result["errors"][0]


def test_doctor_counts_albums_without_recorded_descriptions(tmp_path):
    catalogue(tmp_path)
    assert diagnose(tmp_path)["without_description"] == 1


def test_metadata_rejects_malformed_description_fields(tmp_path):
    catalogue(tmp_path)
    shard = json.loads((tmp_path / "catalog/example.json").read_text(encoding="utf-8"))
    shard["albums"][0]["description"] = "kept"
    shard["albums"][0]["description_truncated"] = True
    validate_metadata(shard, "catalog/example.json")
    for field, bad in (("description", 5), ("description_truncated", "yes")):
        broken = json.loads(json.dumps(shard))
        broken["albums"][0][field] = bad
        with pytest.raises(ValueError):
            validate_metadata(broken, "catalog/example.json")


@pytest.mark.parametrize("price", [
    0, -0.0, 1, 12.5, pytest.param(10**308, id="large-integer"),
    pytest.param(sys.float_info.max, id="largest-float"),
    pytest.param(int(sys.float_info.max), id="largest-float-as-integer"),
    pytest.param(int(sys.float_info.max) + 1, id="integer-rounding-to-largest-float"),
])
def test_metadata_accepts_finite_non_negative_prices(tmp_path, price):
    catalogue(tmp_path)
    shard = json.loads((tmp_path / "catalog/example.json").read_text(encoding="utf-8"))
    shard["albums"][0]["price"] = price
    assert validate_metadata(shard, "catalog/example.json") is shard
    assert shard["albums"][0]["price"] == price


@pytest.mark.parametrize("price", [
    True, False, None, "1", [], {}, -1, -0.1,
    math.nan, math.inf, -math.inf,
    pytest.param(10**400, id="oversized-positive"),
    pytest.param(-(10**400), id="oversized-negative"),
])
def test_metadata_rejects_invalid_prices_consistently(tmp_path, price):
    catalogue(tmp_path)
    shard = json.loads((tmp_path / "catalog/example.json").read_text(encoding="utf-8"))
    shard["albums"][0]["price"] = price
    with pytest.raises(ValueError, match="invalid price"):
        validate_metadata(shard, "catalog/example.json")


@pytest.mark.parametrize("command", ["doctor", "snapshot", "restore"])
@pytest.mark.parametrize("price", [pytest.param(10**400, id="oversized"), -1])
def test_maintenance_reports_invalid_price_without_changing_metadata(
    tmp_path, capsys, command, price,
):
    catalogue(tmp_path)
    files = [tmp_path / "index.json", tmp_path / "catalog/example.json"]
    shard = json.loads(files[1].read_text(encoding="utf-8"))
    shard["albums"][0]["price"] = price
    arguments = ["--data", str(tmp_path), command]
    if command == "restore":
        archive_path = tmp_path / "bad.zip"
        with zipfile.ZipFile(archive_path, "w") as archive:
            archive.writestr("index.json", files[0].read_bytes())
            archive.writestr("catalog/example.json", json.dumps(shard))
        arguments.append(str(archive_path))
    else:
        atomic_write_json(files[1], shard)
    before = {path: path.read_bytes() for path in files}

    assert main(arguments) == 1

    output = capsys.readouterr()
    assert "invalid price" in output.out + output.err
    if command == "doctor":
        assert json.loads(output.out)["issues"]
    assert {path: path.read_bytes() for path in files} == before
    assert not (tmp_path / "snapshots").exists()


def test_load_shard_reports_oversized_price_as_corrupt_metadata(tmp_path):
    catalogue(tmp_path)
    path = tmp_path / "catalog/example.json"
    shard = json.loads(path.read_text(encoding="utf-8"))
    shard["albums"][0]["price"] = 10**400
    atomic_write_json(path, shard)
    with pytest.raises(ValueError, match="corrupt shard .*invalid price"):
        load_shard(tmp_path, "example")


def test_metadata_accepts_missing_price(tmp_path):
    catalogue(tmp_path)
    shard = json.loads((tmp_path / "catalog/example.json").read_text(encoding="utf-8"))
    del shard["albums"][0]["price"]
    assert validate_metadata(shard, "catalog/example.json") is shard
