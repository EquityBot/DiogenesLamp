//! Ported from tests/test_dedup.py.
mod common;

use std::collections::BTreeSet;

use common::{json, lamp, obj, read_json, write};
use lamp::dedup::{duplicate_key, item_keys, newest_listings};
use lamp::pyjson::{self, Map, Style, Value};
use lamp::schema::{public_https_host, validate_metadata};
use lamp::storage::{Shard, load_shard, reindex, write_index, write_shard};

fn album(aid: &str, seller: &str, extra: &str) -> Map {
    let mut map = obj(&json(&format!(
        r#"{{"id": "{aid}", "seller": "{seller}", "title": "A specific product", "thumb": "",
            "album_url": "https://{seller}.x.yupoo.com/albums/{aid}", "links": []}}"#
    )))
    .clone();
    if !extra.is_empty() {
        for (k, v) in obj(&json(extra)) {
            map.insert(k.clone(), v.clone());
        }
    }
    map
}

fn item(aid: &str, seller: &str, extra: &str) -> Map {
    let mut map = album(aid, seller, extra);
    map.insert(
        "links".into(),
        json(r#"[{"platform": "weidian", "url": "https://weidian.com/item.html?itemID=7"}]"#),
    );
    map
}

fn ids(albums: &[Map]) -> Vec<String> {
    albums.iter().map(|a| a["id"].py_str()).collect()
}

fn pairs(aliases: &[(String, String)]) -> Vec<(&str, &str)> {
    aliases
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect()
}

#[test]
fn newest_posting_wins_even_when_older_listing_was_fetched_later() {
    let mut old = item(
        "100",
        "one",
        r#"{"date_published": "2026-01-01", "updated_at": "2026-10-07"}"#,
    );
    let links: Vec<Value> = old["links"]
        .as_array()
        .unwrap()
        .iter()
        .cycle()
        .take(2)
        .cloned()
        .collect();
    old.insert("links".into(), Value::Array(links));
    let new = item(
        "200",
        "two",
        r#"{"date_published": "2026-02-01", "updated_at": "2026-03-01"}"#,
    );
    let (selected, aliases) = newest_listings(&[old.clone(), new]);
    assert_eq!(ids(&selected), ["200"]);
    assert_eq!(pairs(&aliases), [("100", "200")]);
    assert_eq!(old["id"].py_str(), "100");
}

#[test]
fn posting_date_then_exact_large_id_breaks_ties() {
    let earlier = item(
        "9007199254740993",
        "one",
        r#"{"date_published": "2026-01-01"}"#,
    );
    let mut later = item(
        "9007199254740992",
        "one",
        r#"{"date_published": "2026-02-01"}"#,
    );
    assert_eq!(
        ids(&newest_listings(&[earlier.clone(), later.clone()]).0),
        ["9007199254740992"]
    );
    later.insert("date_published".into(), "2026-01-01".into());
    assert_eq!(
        ids(&newest_listings(&[later, earlier]).0),
        ["9007199254740993"]
    );
}

#[test]
fn latest_listing_wins_even_when_marked_gone() {
    let mut old = item("100", "one", r#"{"date_published": "2026-01-01"}"#);
    let new = item(
        "200",
        "one",
        r#"{"date_published": "2026-02-01", "gone_since": "2026-10-01"}"#,
    );
    assert_eq!(
        ids(&newest_listings(&[old.clone(), new.clone()]).0),
        ["200"]
    );
    old.insert("gone_since".into(), "2026-09-01".into());
    assert_eq!(ids(&newest_listings(&[old, new]).0), ["200"]);
}

#[test]
fn titles_keep_sellers_categories_and_generic_items_separate() {
    let repeated: Vec<Map> = (1..4)
        .map(|i| album(&i.to_string(), "one", r#"{"title": "A category label"}"#))
        .collect();
    let same = vec![
        album("10", "one", ""),
        album("11", "one", ""),
        album("12", "two", ""),
    ];
    let generic = vec![
        album("20", "one", r#"{"title": "hat"}"#),
        album("21", "one", r#"{"title": "hat"}"#),
    ];
    assert_eq!(ids(&newest_listings(&repeated).0), ["1", "2", "3"]);
    assert_eq!(ids(&newest_listings(&same).0), ["11", "12"]);
    assert_eq!(ids(&newest_listings(&generic).0), ["20", "21"]);
}

#[test]
fn item_identity_comes_from_unambiguous_marketplace_urls() {
    for (url, expected) in [
        (
            "https://weidian.com/item.html?itemID=7#detail",
            vec!["weidian:7"],
        ),
        ("https://detail.1688.com/offer/42.html", vec!["1688:42"]),
        ("https://item.taobao.com/item.htm?id=7&id=8", vec![]),
        ("https://weidian.com/?userid=7", vec![]),
        ("https://elsewhere.example/item?id=7", vec![]),
        ("https://item.taobao.com:bad/item?id=7", vec![]),
        ("https://m.tb.cn/h.ABC", vec!["url:m.tb.cn/h.ABC"]),
    ] {
        let links = json(&format!(
            r#"[{{"platform": "wrong-label", "url": "{url}"}}]"#
        ));
        let keys = item_keys(links.as_array().unwrap());
        assert_eq!(
            keys,
            expected
                .into_iter()
                .map(String::from)
                .collect::<BTreeSet<_>>(),
            "{url}"
        );
    }
}

#[test]
fn duplicate_keys_match_the_browser_rules() {
    let link = |url: &str| format!(r#"{{"links": [{{"platform": "other", "url": "{url}"}}]}}"#);
    let rows = vec![
        album("1", "one", r#"{"title": "￥450【标准】Model One 样品"}"#),
        item("2", "one", ""),
        album("3", "one", r#"{"title": "hat"}"#),
        album("4", "one", r#"{"title": "Ｉｔｅｍ ＸＹ"}"#),
        album("5", "one", r#"{"title": "四个汉字"}"#),
        album("6", "two", ""),
        album("7", "one", &link("https://detail.1688.com/offer/42.html")),
        album(
            "8",
            "one",
            &link("https://%77eidian.com/item.html?itemID=7"),
        ),
        album("9", "one", &link("https://m.tb.cn/a/../h.ABC")),
        album(
            "10",
            "one",
            &link("https://detail.1688.com/a/%2e%2e/offer/42.html"),
        ),
        album("11", "one", &link("https://m.tb.cn//h.ABC")),
        album("12", "one", &link("https://m.tb.cn/四个汉字")),
        album("13", "one", &link("https://ｔａｏｂａｏ.com/?id=7")),
    ];
    let input = pyjson::dumps(
        &Value::Array(rows.iter().cloned().map(Value::Object).collect()),
        Style::Compact,
    );
    let script = "const d=require('./viewer/dedup.js'); let s=''; process.stdin.on('data',x=>s+=x); process.stdin.on('end',()=>console.log(JSON.stringify(JSON.parse(s).map(d.dupKey))));";
    let js = json(common::run_node(script, &input).trim());
    let rust: Vec<Value> = rows
        .iter()
        .map(|r| duplicate_key(r).map_or(Value::Null, Value::Str))
        .collect();
    assert!(pyjson::py_eq(&js, &Value::Array(rust)), "{js:?}");
}

#[test]
fn newest_selection_matches_the_browser() {
    let groups: Vec<Vec<Map>> = vec![
        vec![
            item(
                "100",
                "one",
                r#"{"date_published": "2026-01-01", "updated_at": "2026-10-07"}"#,
            ),
            item(
                "200",
                "two",
                r#"{"date_published": "2026-02-01", "gone_since": "2026-10-01"}"#,
            ),
        ],
        vec![
            item(
                "9007199254740993",
                "one",
                r#"{"date_published": "2026-01-01"}"#,
            ),
            item(
                "9007199254740992",
                "one",
                r#"{"date_published": "2026-01-01"}"#,
            ),
        ],
        vec![item("250756373", "one", ""), item("260000000", "two", "")],
        vec![
            item("250756373", "one", r#"{"date_published": "2026-02-30"}"#),
            item("260000000", "two", ""),
        ],
        vec![
            item("250756373", "one", r#"{"date_published": "0000-02-01"}"#),
            item("260000000", "two", ""),
        ],
        vec![
            item(
                "250756373",
                "one",
                r#"{"date_published": "2026-10-07T23:00:00-12:00"}"#,
            ),
            item(
                "260000000",
                "two",
                r#"{"date_published": "2026-10-07T01:00:00+12:00"}"#,
            ),
        ],
        vec![
            item("999999999999999999999998", "one", ""),
            item("999999999999999999999999", "two", ""),
        ],
        (1..4)
            .map(|i| album(&i.to_string(), "one", r#"{"title": "Category label"}"#))
            .collect(),
    ];
    let input = pyjson::dumps(
        &Value::Array(
            groups
                .iter()
                .map(|g| Value::Array(g.iter().cloned().map(Value::Object).collect()))
                .collect(),
        ),
        Style::Compact,
    );
    let script = "const d=require('./viewer/dedup.js'); let s=''; process.stdin.on('data',x=>s+=x); process.stdin.on('end',()=>console.log(JSON.stringify(JSON.parse(s).map(rows=>{for(const a of rows)a.dup_key=d.dupKey(a);const r=d.newestListings(rows);return [r.albums.map(a=>a.id),r.aliases]}))));";
    let js = json(common::run_node(script, &input).trim());
    let rust: Vec<Value> = groups
        .iter()
        .map(|g| {
            let (selected, aliases) = newest_listings(g);
            let alias_map: Map = aliases
                .into_iter()
                .map(|(a, b)| (a, Value::Str(b)))
                .collect();
            Value::Array(vec![
                Value::Array(ids(&selected).into_iter().map(Value::Str).collect()),
                Value::Object(alias_map),
            ])
        })
        .collect();
    assert!(pyjson::py_eq(&js, &Value::Array(rust)), "{js:?}");
}

fn shard_of(albums: &[Map]) -> Shard {
    albums
        .iter()
        .map(|a| (a["id"].py_str(), a.clone()))
        .collect()
}

#[test]
fn reindex_cli_is_local_and_invalid_shards_leave_the_manifest_unchanged() {
    let dir = common::tmp();
    let root = dir.path();
    write_shard(
        root,
        "one",
        &shard_of(&[item("100", "one", ""), item("200", "one", "")]),
    )
    .unwrap();
    let index = root.join("index.json");
    write(
        &index,
        r#"{"generated_at": "2026-10-01", "sellers": [{"id": "one", "name": "One", "album_count": 2, "shard": "catalog/one.json"}], "shops": []}"#,
    );
    let data = root.to_string_lossy().into_owned();
    let (code, out, _) = lamp(&["--data", &data, "reindex"], None);
    assert_eq!(code, 0);
    assert_eq!(out.trim(), r#"{"indexed": 1, "duplicates": 1}"#);
    let before = std::fs::read(&index).unwrap();
    write(&root.join("catalog/one.json"), "{invalid");
    let (code, _, err) = lamp(&["--data", &data, "reindex"], None);
    assert_eq!(code, 1);
    assert!(err.contains("corrupt shard"));
    assert_eq!(std::fs::read(&index).unwrap(), before);
    std::fs::remove_file(root.join("catalog/one.json")).unwrap();
    let (code, _, err) = lamp(&["--data", &data, "reindex"], None);
    assert_eq!(code, 1);
    assert!(err.contains("missing shard"));
    assert_eq!(std::fs::read(&index).unwrap(), before);
}

#[test]
fn reindex_rejects_missing_timestamp_and_respects_the_lock() {
    let dir = common::tmp();
    let index = dir.path().join("index.json");
    write(&index, r#"{"sellers": [], "shops": []}"#);
    let before = std::fs::read(&index).unwrap();
    assert!(
        reindex(dir.path())
            .unwrap_err()
            .to_string()
            .contains("generated_at")
    );
    assert_eq!(std::fs::read(&index).unwrap(), before);
    let _lock = lamp::locking::CatalogueLock::acquire(dir.path()).unwrap();
    assert!(matches!(
        reindex(dir.path()),
        Err(lamp::errors::Error::Busy(_))
    ));
}

fn sellers(names: &[&str], extra: &str) -> Vec<Map> {
    names
        .iter()
        .map(|s| {
            obj(&json(&format!(
                r#"{{"id": "{s}", "name": "{s}", "shard": "catalog/{s}.json"{extra}}}"#
            )))
            .clone()
        })
        .collect()
}

#[test]
fn partial_publication_has_no_complete_selection_contract() {
    for unavailable in ["missing", "corrupt"] {
        let dir = common::tmp();
        let root = dir.path();
        write_shard(
            root,
            "one",
            &shard_of(&[item("100", "one", r#"{"date_published": "2026-01-01"}"#)]),
        )
        .unwrap();
        write_shard(
            root,
            "two",
            &shard_of(&[item("200", "two", r#"{"date_published": "2026-02-01"}"#)]),
        )
        .unwrap();
        let list = sellers(&["one", "two", "three"], r#", "album_count": 1"#);
        if unavailable == "corrupt" {
            write(&root.join("catalog/three.json"), "{invalid");
        }
        let original: Vec<Vec<u8>> = ["one", "two"]
            .iter()
            .map(|s| std::fs::read(root.join(format!("catalog/{s}.json"))).unwrap())
            .collect();
        write_index(root, &list, &Value::Array(vec![]), "2026-10-01", false).unwrap();
        let index_path = root.join("index.json");
        let partial = read_json(&index_path);
        validate_metadata(&partial, "index.json").unwrap();
        let p = obj(&partial);
        assert!(!p.contains_key("selection_version"));
        let sellers_out = p["sellers"].as_array().unwrap();
        assert!(pyjson::py_eq(
            &sellers_out[2],
            &Value::Object(list[2].clone())
        ));
        assert_eq!(obj(&sellers_out[1])["album_ids"].py_str(), r#"["200"]"#);
        let before = std::fs::read(&index_path).unwrap();
        assert!(
            reindex(root)
                .unwrap_err()
                .to_string()
                .contains(&format!("{unavailable} shard"))
        );
        assert_eq!(std::fs::read(&index_path).unwrap(), before);
        write_shard(root, "three", &Shard::new()).unwrap();
        write_index(root, &list, &Value::Array(vec![]), "2026-10-01", false).unwrap();
        let complete = read_json(&index_path);
        let c = obj(&complete);
        assert!(matches!(c["selection_version"], Value::Int(1)));
        assert_eq!(c["album_aliases"].py_str(), r#"{"100": "200"}"#);
        assert!(matches!(
            obj(&c["sellers"].as_array().unwrap()[2])["album_count"],
            Value::Int(0)
        ));
        let now: Vec<Vec<u8>> = ["one", "two"]
            .iter()
            .map(|s| std::fs::read(root.join(format!("catalog/{s}.json"))).unwrap())
            .collect();
        assert_eq!(original, now);
    }
}

#[test]
fn reindex_selects_before_seller_filters_and_preserves_captures() {
    let dir = common::tmp();
    let root = dir.path();
    let older = item("100", "one", r#"{"date_published": "2026-01-01"}"#);
    write_shard(root, "one", &shard_of(std::slice::from_ref(&older))).unwrap();
    write_shard(
        root,
        "two",
        &shard_of(&[item("200", "two", r#"{"date_published": "2026-02-01"}"#)]),
    )
    .unwrap();
    write_index(
        root,
        &sellers(&["one", "two"], ""),
        &Value::Array(vec![]),
        "2026-10-01",
        false,
    )
    .unwrap();
    let index = read_json(&root.join("index.json"));
    let list = obj(&index)["sellers"].as_array().unwrap();
    assert_eq!(
        list.iter()
            .map(|s| obj(s)["album_ids"].py_str())
            .collect::<Vec<_>>(),
        [r#"[]"#, r#"["200"]"#]
    );
    assert_eq!(
        list.iter()
            .map(|s| obj(s)["album_count"].py_str())
            .collect::<Vec<_>>(),
        ["0", "1"]
    );
    let before: Vec<Vec<u8>> = ["one", "two"]
        .iter()
        .map(|s| std::fs::read(root.join(format!("catalog/{s}.json"))).unwrap())
        .collect();
    assert_eq!(reindex(root).unwrap(), (1, 1));
    let after: Vec<Vec<u8>> = ["one", "two"]
        .iter()
        .map(|s| std::fs::read(root.join(format!("catalog/{s}.json"))).unwrap())
        .collect();
    assert_eq!(before, after);
    assert!(pyjson::py_eq(
        &Value::Object(load_shard(root, "one").unwrap()["100"].clone()),
        &Value::Object(older)
    ));
    assert!(pyjson::py_eq(&index, &read_json(&root.join("index.json"))));
}

#[test]
fn invalid_index_ids_or_aliases_are_rejected() {
    for (ids, aliases) in [
        (r#"["1", "1"]"#, "{}"),
        ("[1]", "{}"),
        (r#"["bad"]"#, "{}"),
        (r#"["2"]"#, r#"{"1": "3"}"#),
        (r#"["2"]"#, r#"{"2": "2"}"#),
        (r#"["2"]"#, "[]"),
    ] {
        let value = json(&format!(
            r#"{{"sellers": [{{"id": "one", "shard": "catalog/one.json", "album_ids": {ids}}}], "album_aliases": {aliases}}}"#
        ));
        assert!(
            validate_metadata(&value, "index.json").is_err(),
            "{ids} {aliases}"
        );
    }
}

#[test]
fn inadmissible_hostnames_cannot_select_or_alias() {
    for host in [
        "weidi\u{200c}an.com",
        "weidi\u{200d}an.com",
        "m.t\u{200c}b.cn",
        "m.t\u{200d}b.cn",
    ] {
        let path = if host.starts_with("m.") {
            "/h.ABC"
        } else {
            "/item.html?itemID=7"
        };
        let good = if host.starts_with("m.") {
            "m.tb.cn"
        } else {
            "weidian.com"
        };
        let old = album(
            "100",
            "one",
            &format!(
                r#"{{"date_published": "2026-01-01", "links": [{{"platform": "item", "url": "https://{good}{path}"}}]}}"#
            ),
        );
        let bad = album(
            "200",
            "one",
            &format!(
                r#"{{"date_published": "2026-02-01", "links": [{{"platform": "item", "url": "https://{host}{path}"}}]}}"#
            ),
        );
        assert!(item_keys(bad["links"].as_array().unwrap()).is_empty());
        let (selected, aliases) = newest_listings(&[old, bad]);
        assert_eq!(ids(&selected), ["100"]);
        assert!(aliases.is_empty());
    }
}

#[test]
fn ineligible_urls_cannot_compete_through_the_title_fallback() {
    for url in [
        "http://weidian.com/?itemID=7",
        "https://u@weidian.com/?itemID=7",
        "https://weidian.com:bad/?itemID=7",
        "https://weidian.com:65536/?itemID=7",
        "https://%ffweidian.com/?itemID=7",
        "https://weidian.com%2fother/?itemID=7",
        "https://[::1%eth0]/",
        "https://256.1.1.1/",
        "https://1.2.3.999/",
        "https://example.09/",
        "https://1.2.3.4.5/",
        "https://4294967296/",
        "https://exam\u{7f}ple.com/",
        "https://%7fexample.com/",
        "https://xn--a.com/",
        "https://xn--.com/",
        "https://weidi\u{2024}an.com/",
        "https://weidi\u{fe52}an.com/",
    ] {
        let old = item("100", "one", "");
        let bad = album(
            "200",
            "one",
            &format!(
                r#"{{"links": [{{"platform": "item", "url": {}}}]}}"#,
                pyjson::quote(url)
            ),
        );
        let (selected, aliases) = newest_listings(&[old, bad]);
        assert_eq!(ids(&selected), ["100"], "{url}");
        assert!(aliases.is_empty());
    }
}

#[test]
fn reindex_recovers_original_listing_and_doctor_counts_selected_records() {
    for same_shard in [true, false] {
        let dir = common::tmp();
        let root = dir.path();
        let old = item("100", "one", r#"{"date_published": "2026-01-01"}"#);
        let bad_seller = if same_shard { "one" } else { "two" };
        let mut bad = item("200", bad_seller, r#"{"date_published": "2026-02-01"}"#);
        bad.insert(
            "links".into(),
            json(r#"[{"platform": "weidian", "url": "https://weidi‌an.com/?itemID=7"}]"#),
        );
        for row in [&old, &bad] {
            let seller = row["seller"].py_str();
            let mut captures = load_shard(root, &seller).unwrap();
            captures.insert(row["id"].py_str(), row.clone());
            write_shard(root, &seller, &captures).unwrap();
        }
        let mut names: Vec<&str> = vec!["one", bad_seller];
        names.dedup();
        let list: Vec<String> = names
            .iter()
            .map(|s| {
                let is_bad = *s == bad_seller;
                format!(r#"{{"id": "{s}", "name": "{s}", "shard": "catalog/{s}.json", "album_count": {}, "album_ids": {}}}"#,
                    i32::from(is_bad), if is_bad { r#"["200"]"# } else { "[]" })
            })
            .collect();
        write(
            &root.join("index.json"),
            format!(
                r#"{{"sellers": [{}], "shops": [], "generated_at": "2026-10-01", "album_aliases": {{"100": "200"}}}}"#,
                list.join(",")
            ),
        );
        let raw: Vec<Vec<u8>> = names
            .iter()
            .map(|s| std::fs::read(root.join(format!("catalog/{s}.json"))).unwrap())
            .collect();
        let backup = lamp::maintenance::snapshot(root).unwrap();
        assert_eq!(reindex(root).unwrap(), (1, 0));
        let index = read_json(&root.join("index.json"));
        let i = obj(&index);
        assert!(matches!(i["selection_version"], Value::Int(1)));
        assert_eq!(i["album_aliases"].py_str(), "{}");
        assert_eq!(i["generated_at"].py_str(), "2026-10-01");
        let selected: Vec<String> = i["sellers"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|s| {
                obj(s)["album_ids"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(Value::py_str)
                    .collect::<Vec<_>>()
            })
            .collect();
        assert_eq!(selected, ["100"]);
        let now: Vec<Vec<u8>> = names
            .iter()
            .map(|s| std::fs::read(root.join(format!("catalog/{s}.json"))).unwrap())
            .collect();
        assert_eq!(raw, now);
        let report = lamp::maintenance::diagnose(root);
        assert_eq!(report["issues"].py_str(), "[]");
        assert!(matches!(report["unadmitted"], Value::Int(1)));
        lamp::maintenance::restore(root, &backup).unwrap();
        let restored: Vec<Vec<u8>> = names
            .iter()
            .map(|s| std::fs::read(root.join(format!("catalog/{s}.json"))).unwrap())
            .collect();
        assert_eq!(raw, restored);
    }
}

#[test]
fn reindex_refuses_duplicate_ids_across_shards() {
    let dir = common::tmp();
    let root = dir.path();
    for s in ["one", "two"] {
        write_shard(root, s, &shard_of(&[item("100", s, "")])).unwrap();
    }
    let path = root.join("index.json");
    write(
        &path,
        r#"{"sellers": [{"id": "one", "shard": "catalog/one.json"}, {"id": "two", "shard": "catalog/two.json"}], "shops": [], "generated_at": "2026-10-01"}"#,
    );
    let before = std::fs::read(&path).unwrap();
    assert!(
        reindex(root)
            .unwrap_err()
            .to_string()
            .contains("duplicate album ID")
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn canonical_url_admission_matches_viewer_controls() {
    let urls = [
        "https://weidian.com/?itemID=7",
        "https://m.tb.cn/h.ABC",
        "https://%77eidian.com/?itemID=7",
        "https://ｔａｏｂａｏ.com/?id=7",
        "https://例子.com/",
        "https://faß.de/",
        "https://xn--fa-hia.de/",
        "https://نامه\u{200c}ای.ir/",
        "https://[::1]/",
        "https://weidian.com:443/",
        "https://weidi\u{200c}an.com/",
        "https://weidi\u{200d}an.com/",
        "https://m.t\u{200c}b.cn/",
        "https://m.t\u{200d}b.cn/",
        "https://weidian.com:65536/",
        "https://weidian.com:bad/",
        "https://%ffweidian.com/",
        "https://weidian.com%2fother/",
        "https://[::1%eth0]/",
        "https://256.1.1.1/",
        "https://1.2.3.999/",
        "https://example.09/",
        "https://1.2.3.4.5/",
        "https://4294967296/",
        "https://127.1/",
        "https://0x7f.01/",
        "https://2130706433/",
        "https://127.0.0.1./",
        "https://0x/",
        "https://1.0x/",
        "https://exam\u{7f}ple.com/",
        "https://%7fexample.com/",
        "https://weidi\u{2024}an.com/",
        "https://weidi\u{fe52}an.com/",
    ];
    let input = pyjson::dumps(
        &Value::Array(urls.iter().map(|u| Value::Str(u.to_string())).collect()),
        Style::Compact,
    );
    let script = "let s='';process.stdin.on('data',x=>s+=x);process.stdin.on('end',()=>console.log(JSON.stringify(JSON.parse(s).map(raw=>{try{const u=new URL(raw);return u.protocol==='https:'&&!u.username&&!u.password?u.hostname.replace(/^\\[|\\]$/g,''):null;}catch{return null;}}))));";
    let js = json(common::run_node(script, &input).trim());
    let rust: Vec<Value> = urls
        .iter()
        .map(|u| public_https_host(u).map_or(Value::Null, Value::Str))
        .collect();
    let js_items = js.as_array().unwrap();
    for (i, url) in urls.iter().enumerate() {
        assert!(
            pyjson::py_eq(&js_items[i], &rust[i]),
            "{url}: js {:?} rust {:?}",
            js_items[i],
            rust[i]
        );
    }
}
