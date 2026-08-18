from pathlib import Path

import pytest

from lamp.ingest import Source, load_sources, merge_sources, parse_dump, write_sources

DUMP = """\
1\thttps://sellerx.x.yupoo.com/
2\thttps://sellerx.x.yupoo.com/albums
3\thttps://shop00000001.taobao.com/
4\tN/A

5\thttps://weidian.com/?userid=99&spider_token=zz
https://sellery.x.yupoo.com/categories
7,https://sellerz.x.yupoo.com/
"""


def test_parse_dump_strips_indexes_blanks():
    rows = parse_dump(DUMP)
    assert rows == [
        "https://sellerx.x.yupoo.com/",
        "https://sellerx.x.yupoo.com/albums",
        "https://shop00000001.taobao.com/",
        "N/A",
        "https://weidian.com/?userid=99&spider_token=zz",
        "https://sellery.x.yupoo.com/categories",
        "https://sellerz.x.yupoo.com/",
    ]


def test_merge_dedupes_and_counts():
    merged, report = merge_sources({}, parse_dump(DUMP))
    assert set(merged) == {
        "yupoo:sellerx",
        "yupoo:sellery",
        "yupoo:sellerz",
        "shop:taobao-shop00000001",
        "shop:weidian-99",
    }
    assert (report.rows, report.invalid) == (7, 1)
    assert (report.yupoo_new, report.shops_new) == (3, 2)
    assert report.shops_by_platform == {"taobao": 1, "weidian": 1}


def test_merge_is_idempotent_and_preserves_existing():
    first, _ = merge_sources({}, parse_dump(DUMP))
    edited = dict(first)
    sellerx = edited["yupoo:sellerx"]
    edited["yupoo:sellerx"] = type(sellerx)(
        sellerx.kind, sellerx.id, sellerx.platform, sellerx.url, "My Renamed Seller"
    )
    second, report = merge_sources(edited, parse_dump(DUMP))
    assert second == edited  # nothing new, manual rename preserved
    assert report.yupoo_new == 0 and report.shops_new == 0
    assert report.yupoo_known == 3 and report.shops_known == 2


def test_toml_roundtrip(tmp_path: Path):
    merged, _ = merge_sources({}, parse_dump(DUMP))
    path = tmp_path / "sources.toml"
    write_sources(path, merged)
    assert load_sources(path) == merged


def test_load_missing_file_returns_empty(tmp_path: Path):
    assert load_sources(tmp_path / "nope.toml") == {}


def test_write_sources_rejects_unsafe_id(tmp_path: Path):
    bad = Source("shop", "weidian-1.2", "web", "https://weidian.com/?userid=1.2", "weidian 1.2")
    with pytest.raises(ValueError):
        write_sources(tmp_path / "sources.toml", {"shop:weidian-1.2": bad})


def test_skip_flag_round_trips_and_survives_reimport(tmp_path):
    """Commenting an entry out would not last - import rewrites the file."""
    path = tmp_path / "sources.toml"
    write_sources(path, {
        "yupoo:huge": Source("yupoo", "huge", "yupoo", "https://huge.x.yupoo.com", "huge",
                             skip=True),
        "yupoo:keep": Source("yupoo", "keep", "yupoo", "https://keep.x.yupoo.com", "keep"),
    })
    loaded = load_sources(path)
    assert loaded["yupoo:huge"].skip is True
    assert loaded["yupoo:keep"].skip is False

    # re-importing a dump that still contains the skipped seller must not
    # quietly re-enable it
    merged, _ = merge_sources(loaded, ["https://huge.x.yupoo.com", "https://new.x.yupoo.com"])
    assert merged["yupoo:huge"].skip is True
    write_sources(path, merged)
    assert load_sources(path)["yupoo:huge"].skip is True
    assert "skip" not in path.read_text(encoding="utf-8").split("[yupoo.new]")[1]
