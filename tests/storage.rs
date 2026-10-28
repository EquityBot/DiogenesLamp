//! Ported from tests/test_storage.py.
mod common;

use common::{json, obj, read, read_json, write};
use lamp::pyjson::{self, Map, Value};
use lamp::schema::data_path;
use lamp::storage::{
    Shard, atomic_write_json, load_shard, merge_album, read_run, write_index, write_run,
    write_shard,
};

fn record() -> Map {
    obj(&json(
        r#"{"id": "111", "seller": "sellerx", "title": "Item A", "list_title": "Item A",
            "thumb": "thumbs/sellerx/111.jpg", "album_url": "https://sellerx.x.yupoo.com/albums/111",
            "links": [{"platform": "weidian", "url": "https://weidian.com/item.html?itemID=9"}],
            "category": "hoodies", "photo_count": 23,
            "first_seen": "2026-08-01T00:00:00+10:00", "updated_at": "2026-08-01T00:00:00+10:00"}"#,
    ))
    .clone()
}

fn with(base: &Map, changes: &[(&str, Value)]) -> Map {
    let mut map = base.clone();
    for (k, v) in changes {
        map.insert((*k).to_string(), v.clone());
    }
    map
}

#[test]
fn corrupt_run_status_is_reported_and_preserved() {
    let dir = common::tmp();
    let path = dir.path().join("runs/sellerx.json");
    write(&path, r#"{"failed_ids":"101"}"#);
    assert!(
        read_run(dir.path(), "sellerx")
            .unwrap_err()
            .to_string()
            .contains("unreadable run summary")
    );
    assert!(
        write_run(dir.path(), "sellerx", "2026-09-26", Map::new())
            .unwrap_err()
            .to_string()
            .contains("unreadable run summary")
    );
    assert_eq!(read(&path), r#"{"failed_ids":"101"}"#);
    for short in ["-1", "true", r#""5""#] {
        write(&path, format!(r#"{{"short_listing":{short}}}"#));
        assert!(
            read_run(dir.path(), "sellerx")
                .unwrap_err()
                .to_string()
                .contains("invalid short_listing count")
        );
    }
}

#[test]
fn merge_album_first_seen_and_updated_at() {
    let fresh = with(
        &record(),
        &[("first_seen", "".into()), ("updated_at", "".into())],
    );
    let merged = merge_album(None, &fresh, "2026-08-17T12:00:00+10:00");
    assert_eq!(merged["first_seen"].py_str(), "2026-08-17T12:00:00+10:00");
    assert_eq!(merged["updated_at"].py_str(), "2026-08-17T12:00:00+10:00");
    let mut legacy = record();
    legacy.shift_remove("first_seen");
    assert_eq!(
        merge_album(Some(&legacy), &record(), "now")["first_seen"].py_str(),
        "now"
    );
    let fresh = with(
        &record(),
        &[
            ("title", "Item A v2".into()),
            ("first_seen", "".into()),
            ("updated_at", "".into()),
        ],
    );
    let merged = merge_album(Some(&record()), &fresh, "2026-08-17T12:00:00+10:00");
    assert_eq!(merged["first_seen"].py_str(), "2026-08-01T00:00:00+10:00");
    assert_eq!(merged["updated_at"].py_str(), "2026-08-17T12:00:00+10:00");
    assert_eq!(merged["title"].py_str(), "Item A v2");
}

#[test]
fn merge_album_keeps_known_dates_and_prefers_fresh_ones() {
    let known = with(
        &record(),
        &[
            ("date_published", "2025-11-24".into()),
            ("date_modified", "2026-08-16".into()),
        ],
    );
    let fresh = with(
        &record(),
        &[
            ("date_published", "".into()),
            ("date_modified", "".into()),
            ("first_seen", "".into()),
            ("updated_at", "".into()),
        ],
    );
    let merged = merge_album(Some(&known), &fresh, "2026-08-17T12:00:00+10:00");
    assert_eq!(merged["date_published"].py_str(), "2025-11-24");
    assert_eq!(merged["date_modified"].py_str(), "2026-08-16");
    let known = with(
        &record(),
        &[
            ("date_published", "2025-11-24".into()),
            ("date_modified", "2025-11-24".into()),
        ],
    );
    let fresh = with(
        &record(),
        &[
            ("date_published", "2025-11-24".into()),
            ("date_modified", "2026-08-17".into()),
            ("first_seen", "".into()),
            ("updated_at", "".into()),
        ],
    );
    assert_eq!(
        merge_album(Some(&known), &fresh, "x")["date_modified"].py_str(),
        "2026-08-17"
    );
}

#[test]
fn absent_price_does_not_keep_the_old_price() {
    let known = with(&record(), &[("price", Value::Int(180))]);
    let fresh = with(
        &record(),
        &[
            ("price", Value::Int(0)),
            ("first_seen", "".into()),
            ("updated_at", "".into()),
        ],
    );
    let merged = merge_album(Some(&known), &fresh, "2026-08-17T12:00:00+10:00");
    assert!(matches!(merged["price"], Value::Int(0)));
    assert!(
        merged["history"]
            .as_array()
            .unwrap()
            .iter()
            .all(|e| obj(e)["field"].py_str() != "price")
    );
}

#[test]
fn change_history_is_bounded_and_unchanged_scrapes_add_nothing() {
    let mut old = with(&record(), &[("price", Value::Int(180))]);
    for amount in 181..241 {
        old = merge_album(
            Some(&old),
            &with(&record(), &[("price", Value::Int(amount))]),
            &format!("run-{amount}"),
        );
    }
    assert_eq!(old["history"].as_array().unwrap().len(), 50);
    let same = merge_album(
        Some(&old),
        &with(&record(), &[("price", Value::Int(240))]),
        "later",
    );
    assert!(pyjson::py_eq(&same["history"], &old["history"]));
}

#[test]
fn shard_roundtrip_sorted_missing_and_corrupt() {
    let dir = common::tmp();
    let albums: Shard = [("5", "5"), ("10", "10")]
        .iter()
        .map(|(k, id)| (k.to_string(), with(&record(), &[("id", (*id).into())])))
        .collect();
    write_shard(dir.path(), "sellerx", &albums).unwrap();
    let loaded = load_shard(dir.path(), "sellerx").unwrap();
    assert_eq!(loaded.keys().cloned().collect::<Vec<_>>(), ["10", "5"]);
    assert!(load_shard(dir.path(), "ghost").unwrap().is_empty());
    write(&dir.path().join("catalog/bad.json"), "{broken");
    let err = load_shard(dir.path(), "bad").unwrap_err().to_string();
    assert!(
        err.starts_with("corrupt shard") && err.contains("bad.json"),
        "{err}"
    );
    write(
        &dir.path().join("catalog/noid.json"),
        r#"{"albums": [{"title": "x"}]}"#,
    );
    assert!(
        load_shard(dir.path(), "noid")
            .unwrap_err()
            .to_string()
            .contains("corrupt shard")
    );
}

#[test]
fn atomic_write_leaves_no_temporary_file() {
    let dir = common::tmp();
    let target = dir.path().join("x.json");
    atomic_write_json(&target, &json(r#"{"ok": 1}"#)).unwrap();
    assert_eq!(read(&target), r#"{"ok":1}"#);
    let names: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, ["x.json"]);
}

#[cfg(windows)]
#[test]
fn atomic_write_retries_a_transient_windows_lock_and_cleans_up_on_failure() {
    use std::os::windows::fs::OpenOptionsExt;
    let dir = common::tmp();
    let target = dir.path().join("x.json");
    atomic_write_json(&target, &json(r#"{"ok": 1}"#)).unwrap();
    // A reader holding the destination without delete sharing blocks the rename.
    let holder = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(&target)
        .unwrap();
    let releaser = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(250));
        drop(holder);
    });
    atomic_write_json(&target, &json(r#"{"ok": 2}"#)).unwrap();
    releaser.join().unwrap();
    assert_eq!(read(&target), r#"{"ok":2}"#);
    let holder = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(&target)
        .unwrap();
    assert!(atomic_write_json(&target, &json(r#"{"ok": 3}"#)).is_err());
    drop(holder);
    assert_eq!(read(&target), r#"{"ok":2}"#);
    let names: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, ["x.json"]);
}

#[test]
fn concurrent_atomic_writes_use_independent_temporary_files() {
    let dir = common::tmp();
    let target = dir.path().join("shared.json");
    let handles: Vec<_> = ["A", "B"]
        .into_iter()
        .map(|name| {
            let target = target.clone();
            std::thread::spawn(move || {
                atomic_write_json(&target, &json(&format!(r#"{{"writer": "{name}"}}"#)))
            })
        })
        .collect();
    for handle in handles {
        let _ = handle.join().unwrap();
    }
    let writer = obj(&read_json(&target))["writer"].py_str();
    assert!(writer == "A" || writer == "B");
    let names: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, ["shared.json"]);
}

#[test]
fn write_index_selects_and_keeps_captures() {
    let dir = common::tmp();
    let sellers = vec![obj(&json(r#"{"id": "sellerx", "name": "sellerx", "url": "https://sellerx.x.yupoo.com", "album_count": 2, "shard": "catalog/sellerx.json"}"#)).clone()];
    let shops = json(
        r#"[{"id": "weidian-7", "platform": "weidian", "name": "weidian 7", "url": "https://weidian.com/?userid=7"}]"#,
    );
    let shard: Shard = [
        ("111".to_string(), record()),
        ("112".to_string(), with(&record(), &[("id", "112".into())])),
    ]
    .into_iter()
    .collect();
    write_shard(dir.path(), "sellerx", &shard).unwrap();
    write_index(
        dir.path(),
        &sellers,
        &shops,
        "2026-08-17T12:00:00+10:00",
        false,
    )
    .unwrap();
    let index = read_json(&dir.path().join("index.json"));
    let i = obj(&index);
    assert_eq!(i["generated_at"].py_str(), "2026-08-17T12:00:00+10:00");
    let mut expected = sellers[0].clone();
    expected.insert("album_count".into(), Value::Int(1));
    expected.insert("album_ids".into(), json(r#"["112"]"#));
    assert!(pyjson::py_eq(
        &i["sellers"],
        &Value::Array(vec![Value::Object(expected)])
    ));
    assert_eq!(i["album_aliases"].py_str(), r#"{"111": "112"}"#);
    assert_eq!(load_shard(dir.path(), "sellerx").unwrap().len(), 2);
    assert!(pyjson::py_eq(&i["shops"], &shops));
}

#[test]
fn data_path_contains_and_works_before_the_directory_exists() {
    let dir = common::tmp();
    let missing = dir.path().join("missing");
    let resolved = lamp::pyerr::resolve(&missing);
    assert_eq!(
        data_path(&missing, "index.json", false).unwrap(),
        resolved.join("index.json")
    );
    assert!(data_path(dir.path(), "runs/../index.json", false).is_err());
    let root = dir.path().join("data");
    std::fs::create_dir_all(&root).unwrap();
    let outside = dir.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    #[cfg(windows)]
    let linked = std::os::windows::fs::symlink_dir(&outside, root.join("catalog"));
    #[cfg(not(windows))]
    let linked = std::os::unix::fs::symlink(&outside, root.join("catalog"));
    if linked.is_ok() {
        let err = data_path(&root, "catalog/sellerx.json", false)
            .unwrap_err()
            .to_string();
        assert!(err.contains("escapes data directory"), "{err}");
    }
}

#[test]
fn data_path_survives_a_sibling_creating_its_directory() {
    let dir = common::tmp();
    let root = dir.path().to_path_buf();
    let expected = lamp::pyerr::resolve(&root)
        .join("catalog")
        .join("sellery.json");
    let creator = {
        let root = root.clone();
        std::thread::spawn(move || std::fs::create_dir_all(root.join("catalog")).unwrap())
    };
    for _ in 0..50 {
        assert_eq!(
            data_path(&root, "catalog/sellery.json", false).unwrap(),
            expected
        );
    }
    creator.join().unwrap();
    assert_eq!(
        data_path(&root, "catalog/sellery.json", false).unwrap(),
        expected
    );
}
