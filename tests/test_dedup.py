import json
import shutil
import subprocess
from pathlib import Path

import pytest

from lamp.cli import main
from lamp.dedup import duplicate_key, item_keys, newest_listings
from lamp.schema import validate_metadata
from lamp.storage import load_shard, reindex, write_index, write_shard


def album(aid, seller="one", **values):
    return {"id": aid, "seller": seller, "title": "A specific product", "thumb": "",
            "album_url": f"https://{seller}.x.yupoo.com/albums/{aid}", "links": [], **values}


def item(aid, seller="one", **values):
    return album(aid, seller, links=[{"platform": "weidian",
                 "url": "https://weidian.com/item.html?itemID=7"}], **values)


def test_newest_posting_wins_even_when_older_listing_was_fetched_later():
    old = item("100", date_published="2026-01-01", updated_at="2026-10-07")
    old["links"] *= 2
    new = item("200", "two", date_published="2026-02-01", updated_at="2026-03-01")
    selected, aliases = newest_listings([old, new])
    assert selected == [new]
    assert aliases == {"100": "200"}
    assert old["id"] == "100"


def test_posting_date_then_exact_large_id_breaks_ties():
    earlier = item("9007199254740993", date_published="2026-01-01")
    later = item("9007199254740992", date_published="2026-02-01")
    assert newest_listings([earlier, later])[0] == [later]
    later["date_published"] = "2026-01-01"
    assert newest_listings([later, earlier])[0] == [earlier]


def test_latest_listing_wins_even_when_marked_gone():
    old = item("100", date_published="2026-01-01")
    new = item("200", date_published="2026-02-01", gone_since="2026-10-01")
    assert newest_listings([old, new])[0] == [new]
    old["gone_since"] = "2026-09-01"
    assert newest_listings([old, new])[0] == [new]


def test_titles_keep_sellers_categories_and_generic_items_separate():
    repeated = [album(str(i), title="A category label") for i in range(1, 4)]
    same_title = [album("10"), album("11"), album("12", "two")]
    generic = [album("20", title="hat"), album("21", title="hat")]
    assert newest_listings(repeated)[0] == repeated
    assert newest_listings(same_title)[0] == same_title[1:]
    assert newest_listings(generic)[0] == generic


@pytest.mark.parametrize("url,expected", [
    ("https://weidian.com/item.html?itemID=7#detail", {"weidian:7"}),
    ("https://detail.1688.com/offer/42.html", {"1688:42"}),
    ("https://item.taobao.com/item.htm?id=7&id=8", set()),
    ("https://weidian.com/?userid=7", set()),
    ("https://elsewhere.example/item?id=7", set()),
    ("https://item.taobao.com:bad/item?id=7", set()),
    ("https://m.tb.cn/h.ABC", {"url:m.tb.cn/h.ABC"}),
])
def test_item_identity_comes_from_unambiguous_marketplace_urls(url, expected):
    assert item_keys([{"platform": "wrong-label", "url": url}]) == expected


def test_duplicate_keys_match_the_browser_rules():
    from pathlib import Path

    node = shutil.which("node")
    assert node, "Node-backed parity coverage must run"
    rows = [album("1", title="￥450【标准】Model One 样品"), item("2"),
            album("3", title="hat"), album("4", title="Ｉｔｅｍ ＸＹ"),
            album("5", title="四个汉字"), album("6", "two"),
            album("7", links=[{"platform": "other", "url": "https://detail.1688.com/offer/42.html"}]),
            album("8", links=[{"platform": "other", "url": "https://%77eidian.com/item.html?itemID=7"}]),
            album("9", links=[{"platform": "other", "url": "https://m.tb.cn/a/../h.ABC"}]),
            album("10", links=[{"platform": "other", "url": "https://detail.1688.com/a/%2e%2e/offer/42.html"}]),
            album("11", links=[{"platform": "other", "url": "https://m.tb.cn//h.ABC"}]),
            album("12", links=[{"platform": "other", "url": "https://m.tb.cn/四个汉字"}]),
            album("13", links=[{"platform": "other", "url": "https://ｔａｏｂａｏ.com/?id=7"}])]
    root = Path(__file__).resolve().parents[1]
    script = "const d=require('./viewer/dedup.js'); let s=''; process.stdin.on('data',x=>s+=x); process.stdin.on('end',()=>console.log(JSON.stringify(JSON.parse(s).map(d.dupKey))));"
    result = subprocess.run([node, "-e", script], input=json.dumps(rows), text=True,
                            capture_output=True, cwd=root, check=True)
    assert json.loads(result.stdout) == [duplicate_key(row) for row in rows]


def test_newest_selection_matches_the_browser_with_dates_estimates_and_large_ids():
    from pathlib import Path

    groups = [
        [item("100", date_published="2026-01-01", updated_at="2026-10-07"),
         item("200", "two", date_published="2026-02-01", gone_since="2026-10-01")],
        [item("9007199254740993", date_published="2026-01-01"),
         item("9007199254740992", date_published="2026-01-01")],
        [item("250756373"), item("260000000", "two")],
        [item("250756373", date_published="2026-02-30"), item("260000000", "two")],
        [item("250756373", date_published="0000-02-01"), item("260000000", "two")],
        [item("250756373", date_published="2026-10-07T23:00:00-12:00"),
         item("260000000", "two", date_published="2026-10-07T01:00:00+12:00")],
        [item("999999999999999999999998"), item("999999999999999999999999", "two")],
        [album(str(i), title="Category label") for i in range(1, 4)],
    ]
    node = shutil.which("node")
    assert node, "Node-backed selection parity coverage must run"
    script = "const d=require('./viewer/dedup.js'); let s=''; process.stdin.on('data',x=>s+=x); process.stdin.on('end',()=>console.log(JSON.stringify(JSON.parse(s).map(rows=>{for(const a of rows)a.dup_key=d.dupKey(a);const r=d.newestListings(rows);return [r.albums.map(a=>a.id),r.aliases]}))));"
    result = subprocess.run([node, "-e", script], input=json.dumps(groups), text=True,
                            capture_output=True, cwd=Path(__file__).resolve().parents[1], check=True)
    expected = []
    for group in groups:
        selected, aliases = newest_listings(group)
        expected.append([[a["id"] for a in selected], aliases])
    assert json.loads(result.stdout) == expected


def test_reindex_cli_is_local_and_invalid_shards_leave_the_manifest_unchanged(tmp_path, capsys):
    old, new = item("100"), item("200")
    write_shard(tmp_path, "one", {"100": old, "200": new})
    index_path = tmp_path / "index.json"
    legacy = {"generated_at": "2026-10-01", "sellers": [
        {"id": "one", "name": "One", "album_count": 2, "shard": "catalog/one.json"}], "shops": []}
    index_path.write_text(json.dumps(legacy))
    assert main(["--data", str(tmp_path), "reindex"]) == 0
    assert json.loads(capsys.readouterr().out) == {"indexed": 1, "duplicates": 1}
    before = index_path.read_bytes()
    (tmp_path / "catalog" / "one.json").write_text("{invalid")
    assert main(["--data", str(tmp_path), "reindex"]) == 1
    assert "corrupt shard" in capsys.readouterr().err
    assert index_path.read_bytes() == before
    (tmp_path / "catalog" / "one.json").unlink()
    assert main(["--data", str(tmp_path), "reindex"]) == 1
    assert "missing shard" in capsys.readouterr().err
    assert index_path.read_bytes() == before


def test_reindex_rejects_missing_timestamp_and_respects_the_catalogue_lock(tmp_path):
    from lamp.locking import CatalogueBusyError, catalogue_lock

    index_path = tmp_path / "index.json"
    index_path.write_text(json.dumps({"sellers": [], "shops": []}))
    before = index_path.read_bytes()
    with pytest.raises(ValueError, match="generated_at"):
        reindex(tmp_path)
    assert index_path.read_bytes() == before
    with catalogue_lock(tmp_path), pytest.raises(CatalogueBusyError):
        reindex(tmp_path)


@pytest.mark.parametrize("unavailable", ["missing", "corrupt", "unreadable"])
def test_partial_publication_has_no_complete_selection_contract(tmp_path, unavailable, monkeypatch):
    write_shard(tmp_path, "one", {"100": item("100", date_published="2026-01-01")})
    write_shard(tmp_path, "two", {"200": item("200", "two", date_published="2026-02-01")})
    sellers = [{"id": s, "name": s, "shard": f"catalog/{s}.json", "album_count": 1}
               for s in ("one", "two", "three")]
    missing = tmp_path / "catalog" / "three.json"
    if unavailable == "corrupt":
        missing.write_text("{invalid")
    if unavailable == "unreadable":
        write_shard(tmp_path, "three", {})
        unreadable_bytes = missing.read_bytes()
        read_text = Path.read_text

        def fail_shard_read(path, *args, **kwargs):
            if path == missing:
                raise PermissionError("fabricated shard read failure")
            return read_text(path, *args, **kwargs)

        monkeypatch.setattr(Path, "read_text", fail_shard_read)
    original = {s: (tmp_path / "catalog" / f"{s}.json").read_bytes() for s in ("one", "two")}
    write_index(tmp_path, sellers, [], "2026-10-01")
    index_path = tmp_path / "index.json"
    partial = validate_metadata(json.loads(index_path.read_text()), "index.json")
    assert "selection_version" not in partial
    assert partial["sellers"][2] == sellers[2]
    assert partial["sellers"][1]["album_ids"] == ["200"]
    before = index_path.read_bytes()
    error_kind = "missing" if unavailable == "missing" else "corrupt"
    with pytest.raises(ValueError, match=f"{error_kind} shard"):
        reindex(tmp_path)
    assert index_path.read_bytes() == before
    assert missing.exists() == (unavailable != "missing")
    if unavailable == "unreadable":
        from lamp.ingest import Source
        from lamp.scrape import _write_index

        sources = [Source(kind="seller", id=s, platform="yupoo",
                          url=f"https://{s}.x.yupoo.com/albums", name=s)
                   for s in ("one", "two", "three")]
        _write_index(tmp_path, sources, [], "2026-10-01")
        partial = validate_metadata(json.loads(index_path.read_text()), "index.json")
        assert "selection_version" not in partial
        assert [s["id"] for s in partial["sellers"]] == ["one", "two", "three"]
        assert partial["sellers"][1]["album_ids"] == ["200"]
        assert missing.read_bytes() == unreadable_bytes
        monkeypatch.setattr(Path, "read_text", read_text)
    write_shard(tmp_path, "three", {})
    write_index(tmp_path, sellers, [], "2026-10-01")
    complete = validate_metadata(json.loads(index_path.read_text()), "index.json")
    assert complete["selection_version"] == 1
    assert complete["album_aliases"] == {"100": "200"}
    assert complete["sellers"][2]["album_count"] == 0
    assert original == {s: (tmp_path / "catalog" / f"{s}.json").read_bytes() for s in ("one", "two")}


def test_reindex_selects_before_seller_filters_and_preserves_captures(tmp_path):
    older, newer = item("100", date_published="2026-01-01"), item("200", "two", date_published="2026-02-01")
    write_shard(tmp_path, "one", {"100": older})
    write_shard(tmp_path, "two", {"200": newer})
    sellers = [{"id": s, "name": s, "shard": f"catalog/{s}.json"} for s in ("one", "two")]
    write_index(tmp_path, sellers, [], "2026-10-01")
    index = validate_metadata(json.loads((tmp_path / "index.json").read_text()), "index.json")
    assert [s["album_ids"] for s in index["sellers"]] == [[], ["200"]]
    assert [s["album_count"] for s in index["sellers"]] == [0, 1]
    assert index["album_aliases"] == {"100": "200"}
    before = {s: (tmp_path / "catalog" / f"{s}.json").read_bytes() for s in ("one", "two")}
    assert reindex(tmp_path) == {"indexed": 1, "duplicates": 1}
    assert before == {s: (tmp_path / "catalog" / f"{s}.json").read_bytes() for s in ("one", "two")}
    assert load_shard(tmp_path, "one") == {"100": older}
    assert index == json.loads((tmp_path / "index.json").read_text())


@pytest.mark.parametrize("ids,aliases", [
    (["1", "1"], {}), ([1], {}), (["bad"], {}),
    (["2"], {"1": "3"}), (["2"], {"2": "2"}), (["2"], []),
])
def test_invalid_index_ids_or_aliases_are_rejected(ids, aliases):
    with pytest.raises(ValueError):
        validate_metadata({"sellers": [{"id": "one", "shard": "catalog/one.json", "album_ids": ids}],
                           "album_aliases": aliases}, "index.json")


@pytest.mark.parametrize("host", ["weidi\u200can.com", "weidi\u200dan.com", "m.t\u200cb.cn", "m.t\u200db.cn"])
def test_inadmissible_hostname_cannot_select_or_alias_a_valid_listing(host):
    path = "/h.ABC" if host.startswith("m.") else "/item.html?itemID=7"
    good_host = "m.tb.cn" if host.startswith("m.") else "weidian.com"
    old = album("100", date_published="2026-01-01", links=[
        {"platform": "item", "url": f"https://{good_host}{path}"}])
    bad = album("200", date_published="2026-02-01", links=[
        {"platform": "item", "url": f"https://{host}{path}"}])
    assert item_keys(bad["links"]) == set()
    assert newest_listings([old, bad]) == ([old], {})


@pytest.mark.parametrize("url", [
    "http://weidian.com/?itemID=7", "https://u@weidian.com/?itemID=7",
    "https://weidian.com:bad/?itemID=7", "https://weidian.com:65536/?itemID=7",
    "https://%ffweidian.com/?itemID=7", "https://weidian.com%2fother/?itemID=7",
    "https://[::1%eth0]/", "https://256.1.1.1/", "https://1.2.3.999/",
    "https://example.09/", "https://1.2.3.4.5/", "https://4294967296/",
    "https://exam\x7fple.com/", "https://%7fexample.com/", "https://xn--a.com/", "https://xn--.com/",
    "https://weidi\u2024an.com/", "https://weidi\ufe52an.com/",
])
def test_ineligible_url_cannot_compete_through_the_title_fallback(url):
    old, bad = item("100"), album("200", links=[{"platform": "item", "url": url}])
    assert newest_listings([old, bad]) == ([old], {})


@pytest.mark.parametrize("same_shard", [True, False])
def test_reindex_recovers_original_listing_and_doctor_counts_selected_records(tmp_path, same_shard):
    from lamp.maintenance import diagnose, restore, snapshot

    old = item("100", date_published="2026-01-01")
    bad = item("200", "one" if same_shard else "two", date_published="2026-02-01")
    bad["links"][0]["url"] = "https://weidi\u200can.com/?itemID=7"
    for row in (old, bad):
        captures = load_shard(tmp_path, row["seller"])
        captures[row["id"]] = row
        write_shard(tmp_path, row["seller"], captures)
    sellers = [{"id": s, "name": s, "shard": f"catalog/{s}.json", "album_count": int(s == bad["seller"]),
                "album_ids": ["200"] if s == bad["seller"] else []}
               for s in sorted({old["seller"], bad["seller"]})]
    (tmp_path / "index.json").write_text(json.dumps({"sellers": sellers, "shops": [],
        "generated_at": "2026-10-01", "album_aliases": {"100": "200"}}))
    raw = {s["id"]: (tmp_path / s["shard"]).read_bytes() for s in sellers}
    backup = snapshot(tmp_path)
    assert reindex(tmp_path) == {"indexed": 1, "duplicates": 0}
    index = json.loads((tmp_path / "index.json").read_text())
    assert index["selection_version"] == 1
    assert index["album_aliases"] == {}
    assert index["generated_at"] == "2026-10-01"
    assert [i for s in index["sellers"] for i in s["album_ids"]] == ["100"]
    assert raw == {s["id"]: (tmp_path / s["shard"]).read_bytes() for s in sellers}
    assert diagnose(tmp_path)["issues"] == []
    assert diagnose(tmp_path)["unadmitted"] == 1
    restore(tmp_path, backup)
    assert raw == {s["id"]: (tmp_path / s["shard"]).read_bytes() for s in sellers}


def test_reindex_refuses_duplicate_ids_across_shards_without_publication(tmp_path):
    for s in ("one", "two"):
        write_shard(tmp_path, s, {"100": item("100", s)})
    path = tmp_path / "index.json"
    path.write_text(json.dumps({"sellers": [{"id": s, "shard": f"catalog/{s}.json"}
        for s in ("one", "two")], "shops": [], "generated_at": "2026-10-01"}))
    before = path.read_bytes()
    with pytest.raises(ValueError, match="duplicate album ID"):
        reindex(tmp_path)
    assert path.read_bytes() == before


def test_canonical_url_admission_matches_viewer_controls():
    from pathlib import Path

    from lamp.schema import public_https_host

    urls = ["https://weidian.com/?itemID=7", "https://m.tb.cn/h.ABC",
            "https://%77eidian.com/?itemID=7", "https://ｔａｏｂａｏ.com/?id=7",
            "https://例子.com/", "https://faß.de/", "https://xn--fa-hia.de/",
            "https://نامه\u200cای.ir/", "https://[::1]/", "https://weidian.com:443/",
            "https://weidi\u200can.com/", "https://weidi\u200dan.com/",
            "https://m.t\u200cb.cn/", "https://m.t\u200db.cn/",
            "https://weidian.com:65536/", "https://weidian.com:bad/",
            "https://%ffweidian.com/", "https://weidian.com%2fother/",
            "https://[::1%eth0]/", "https://256.1.1.1/", "https://1.2.3.999/", "https://example.09/",
            "https://1.2.3.4.5/", "https://4294967296/", "https://127.1/", "https://0x7f.01/",
            "https://2130706433/", "https://127.0.0.1./", "https://0x/", "https://1.0x/",
            "https://exam\x7fple.com/", "https://%7fexample.com/",
            "https://weidi\u2024an.com/", "https://weidi\ufe52an.com/"]
    node = shutil.which("node")
    assert node, "Node-backed URL admission coverage must run"
    script = "let s='';process.stdin.on('data',x=>s+=x);process.stdin.on('end',()=>console.log(JSON.stringify(JSON.parse(s).map(raw=>{try{const u=new URL(raw);return u.protocol==='https:'&&!u.username&&!u.password?u.hostname.replace(/^\\[|\\]$/g,''):null;}catch{return null;}}))));"
    result = subprocess.run([node, "-e", script], input=json.dumps(urls), text=True,
                            capture_output=True, check=True, cwd=Path(__file__).resolve().parents[1])
    actual = []
    for url in urls:
        try:
            actual.append(public_https_host(url))
        except ValueError:
            actual.append(None)
    assert actual == json.loads(result.stdout)
