//! Ported from tests/test_maintenance.py.
mod common;

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use common::{json, lamp, obj, read_json, write};
use lamp::covers::{cover_facts, resize_cover};
use lamp::maintenance::{diagnose, prepare_covers, restore, snapshot};
use lamp::pyjson::{self, Style, Value};
use lamp::schema::{data_path, validate_metadata};
use lamp::storage::{atomic_write_json, load_shard};

fn catalogue(root: &Path) {
    atomic_write_json(&root.join("index.json"), &json(r#"{"schema_version": 1, "sellers": [{"id": "example", "shard": "catalog/example.json", "album_count": 1}]}"#)).unwrap();
    atomic_write_json(
        &root.join("catalog/example.json"),
        &json(r#"{"seller": "example", "albums": [{"id": "123", "seller": "example", "title": "Example", "price": 0,
            "album_url": "https://example.x.yupoo.com/albums/123", "links": [], "thumb": "thumbs/example/123.jpg"}]}"#),
    )
    .unwrap();
}

fn tree(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    fn walk(dir: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut entries: Vec<_> = entries.map(|e| e.unwrap().path()).collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                walk(&path, out);
            } else {
                out.push((path.clone(), std::fs::read(&path).unwrap()));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, &mut out);
    out
}

fn zip_member(path: &Path, name: &str) -> Vec<u8> {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    let mut data = Vec::new();
    archive
        .by_name(name)
        .unwrap()
        .read_to_end(&mut data)
        .unwrap();
    data
}

fn make_zip(path: &Path, members: &[(&str, Vec<u8>)]) {
    let mut writer = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    for (name, data) in members {
        writer
            .start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(data).unwrap();
    }
    writer.finish().unwrap();
}

#[test]
fn doctor_is_read_only_and_reports_coverage() {
    let dir = common::tmp();
    catalogue(dir.path());
    let before = tree(dir.path());
    let result = diagnose(dir.path());
    assert_eq!(result["issues"].py_str(), "[]");
    for key in ["albums", "missing_covers", "unpriced"] {
        assert!(matches!(result[key], Value::Int(1)), "{key}");
    }
    assert_eq!(before, tree(dir.path()));
    let (code, _, _) = lamp(&["--data", &dir.path().to_string_lossy(), "doctor"], None);
    assert_eq!(code, 0);
}

#[test]
fn snapshot_restores_corrupt_metadata_and_keeps_previous_bytes() {
    let dir = common::tmp();
    catalogue(dir.path());
    let archive = snapshot(dir.path()).unwrap();
    let shard = dir.path().join("catalog/example.json");
    let original = std::fs::read(&shard).unwrap();
    write(&shard, "corrupt");
    let previous = restore(dir.path(), &archive).unwrap().unwrap();
    assert_eq!(std::fs::read(&shard).unwrap(), original);
    assert_eq!(zip_member(&previous, "catalog/example.json"), b"corrupt");
    assert_eq!(diagnose(dir.path())["issues"].py_str(), "[]");
}

#[test]
fn snapshot_rejects_a_missing_indexed_shard() {
    let dir = common::tmp();
    catalogue(dir.path());
    std::fs::remove_file(dir.path().join("catalog/example.json")).unwrap();
    assert!(
        snapshot(dir.path())
            .unwrap_err()
            .to_string()
            .contains("catalog/example.json: missing indexed shard")
    );
    assert!(!dir.path().join("snapshots").exists());
}

#[test]
fn unsafe_or_incomplete_snapshot_never_changes_metadata() {
    for (name, payload) in [
        ("../outside.json", "{}"),
        ("catalog/../../outside.json", "{}"),
        ("catalog/C:bad.json", "{}"),
        ("index.json", r#"{"schema_version": 99}"#),
        (
            "index.json",
            r#"{"sellers": [{"id": "example", "shard": "catalog/example.json"}]}"#,
        ),
    ] {
        let dir = common::tmp();
        catalogue(dir.path());
        let before = std::fs::read(dir.path().join("index.json")).unwrap();
        let bad = dir.path().join("bad.zip");
        make_zip(&bad, &[(name, payload.as_bytes().to_vec())]);
        assert!(
            matches!(
                restore(dir.path(), &bad),
                Err(lamp::errors::Error::Value(_))
            ),
            "{name}"
        );
        assert_eq!(
            std::fs::read(dir.path().join("index.json")).unwrap(),
            before
        );
        assert!(!dir.path().join("snapshots").exists());
    }
}

#[test]
fn restore_rejects_metadata_the_viewer_cannot_use() {
    let cases: Vec<(&str, &str, Option<&str>)> = vec![
        ("shard", "seller", Some(r#""other""#)),
        ("shard", "seller", None),
        ("album", "seller", Some(r#""other""#)),
        ("album", "seller", None),
        ("album", "title", None),
        ("album", "album_url", None),
        ("album", "thumb", None),
        ("album", "links", None),
        (
            "album",
            "links",
            Some(r#"[{"url": "https://example.com/item"}]"#),
        ),
        (
            "album",
            "links",
            Some(r#"[{"platform": "shop", "url": ""}]"#),
        ),
        ("album", "price_evidence", Some(r#"{"candidates": {}}"#)),
        ("shard", "schema_version", Some("true")),
    ];
    for (target, field, value) in cases {
        let dir = common::tmp();
        catalogue(dir.path());
        let files = [
            dir.path().join("index.json"),
            dir.path().join("catalog/example.json"),
        ];
        let before: Vec<Vec<u8>> = files.iter().map(|p| std::fs::read(p).unwrap()).collect();
        let mut shard = read_json(&files[1]);
        let record = if target == "shard" {
            shard.as_object_mut().unwrap()
        } else {
            shard
                .as_object_mut()
                .unwrap()
                .get_mut("albums")
                .unwrap()
                .as_array_mut()
                .unwrap()[0]
                .as_object_mut()
                .unwrap()
        };
        match value {
            None => {
                record.shift_remove(field);
            }
            Some(v) => {
                record.insert(field.into(), json(v));
            }
        }
        let bad = dir.path().join("bad.zip");
        make_zip(
            &bad,
            &[
                ("index.json", before[0].clone()),
                (
                    "catalog/example.json",
                    pyjson::dumps(&shard, Style::Default).into_bytes(),
                ),
            ],
        );
        assert!(restore(dir.path(), &bad).is_err(), "{target} {field}");
        let after: Vec<Vec<u8>> = files.iter().map(|p| std::fs::read(p).unwrap()).collect();
        assert_eq!(before, after);
        assert!(!dir.path().join("snapshots").exists());
    }
}

#[test]
fn schema_rejects_duplicate_ids_unsafe_covers_and_links() {
    let dir = common::tmp();
    catalogue(dir.path());
    let shard = read_json(&dir.path().join("catalog/example.json"));
    let original = obj(&shard)["albums"].as_array().unwrap()[0].clone();
    for (key, value) in [
        ("thumb", r#""../outside.jpg""#),
        ("price", "NaN"),
        (
            "links",
            r#"[{"platform": "shop", "url": "javascript:alert(1)"}]"#,
        ),
    ] {
        let mut album = original.clone();
        album
            .as_object_mut()
            .unwrap()
            .insert(key.into(), json(value));
        let value = Value::Object(lamp::obj! { "seller" => "example", "albums" => vec![album] });
        assert!(
            validate_metadata(&value, "catalog/example.json").is_err(),
            "{key}"
        );
    }
    let value = Value::Object(
        lamp::obj! { "seller" => "example", "albums" => vec![original.clone(), original] },
    );
    assert!(validate_metadata(&value, "catalog/example.json").is_err());
    assert!(data_path(dir.path(), "runs/../index.json", false).is_err());
}

#[test]
fn cover_conversion_is_bounded_oriented_and_resumable() {
    let dir = common::tmp();
    catalogue(dir.path());
    let cover = dir.path().join("thumbs/example/123.jpg");
    let jpeg = common::jpeg_with_orientation(1600, 800, 6);
    write(&cover, &jpeg);
    let result = prepare_covers(dir.path(), 1).unwrap();
    assert!(
        matches!(result["resized"], Value::Int(1)) && matches!(result["processed"], Value::Int(1))
    );
    let after = match result["after_bytes"] {
        Value::Int(n) => n,
        _ => 0,
    };
    assert!(after < jpeg.len() as i128);
    let opened = lamp::covers::open(&cover).unwrap();
    assert_eq!(
        (opened.width, opened.height, opened.orientation),
        (320, 640, 1)
    );
    assert!(matches!(
        prepare_covers(dir.path(), 1).unwrap()["processed"],
        Value::Int(0)
    ));
    let facts = cover_facts(&cover);
    assert!(pyjson::py_eq(
        &Value::Object(facts.clone()),
        &Value::Object(cover_facts(&cover))
    ));
    assert_eq!(facts["thumb_version"].py_str().len(), 24);
    let stored = read_json(&dir.path().join("catalog/example.json"));
    assert_eq!(
        obj(&obj(&stored)["albums"].as_array().unwrap()[0])["thumb_version"].py_str(),
        facts["thumb_version"].py_str()
    );
}

#[test]
fn transparency_survives_and_failed_conversion_preserves_original() {
    let dir = common::tmp();
    let png = dir.path().join("alpha.png");
    let image = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
        1000,
        800,
        image::Rgba([20, 40, 60, 80]),
    ));
    write(&png, common::encode(&image, image::ImageFormat::Png));
    resize_cover(&png).unwrap();
    let decoded = image::open(&png).unwrap();
    assert!(matches!(decoded, image::DynamicImage::ImageRgba8(_)));
    assert_eq!(decoded.to_rgba8().get_pixel(0, 0).0[3], 80);
    write(&png, b"not an image");
    assert!(matches!(
        resize_cover(&png),
        Err(lamp::errors::Error::Os(_))
    ));
    assert_eq!(std::fs::read(&png).unwrap(), b"not an image");
}

#[test]
fn restore_preserves_orphaned_shards_when_index_is_missing() {
    let dir = common::tmp();
    catalogue(dir.path());
    let archive = snapshot(dir.path()).unwrap();
    std::fs::remove_file(dir.path().join("index.json")).unwrap();
    write(&dir.path().join("catalog/example.json"), "orphaned bytes");
    let previous = restore(dir.path(), &archive).unwrap().unwrap();
    assert_eq!(
        zip_member(&previous, "catalog/example.json"),
        b"orphaned bytes"
    );
}

#[test]
fn cover_fingerprints_tolerate_brightness_but_distinguish_geometry() {
    let dir = common::tmp();
    let mut base = image::RgbImage::from_pixel(256, 256, image::Rgb([255, 255, 255]));
    for y in 22..=178 {
        for x in 12..=83 {
            base.put_pixel(x, y, image::Rgb([0, 0, 0]));
        }
    }
    for y in 73..=221u32 {
        for x in 107..=236u32 {
            let (cx, cy, rx, ry) = (171.5f64, 147.0f64, 64.5f64, 74.0f64);
            if ((f64::from(x) - cx) / rx).powi(2) + ((f64::from(y) - cy) / ry).powi(2) <= 1.0 {
                base.put_pixel(x, y, image::Rgb([128, 128, 128]));
            }
        }
    }
    let darker = image::RgbImage::from_fn(256, 256, |x, y| {
        let p = base.get_pixel(x, y).0;
        image::Rgb(p.map(|v| (f64::from(v) * 0.85) as u8))
    });
    let mirrored = image::imageops::flip_horizontal(&base);
    let hashes: Vec<u64> = [base, darker, mirrored]
        .into_iter()
        .enumerate()
        .map(|(i, img)| {
            let path = dir.path().join(format!("sample{i}.png"));
            write(
                &path,
                common::encode(
                    &image::DynamicImage::ImageRgb8(img),
                    image::ImageFormat::Png,
                ),
            );
            u64::from_str_radix(&cover_facts(&path)["cover_hash"].py_str(), 16).unwrap()
        })
        .collect();
    assert!((hashes[0] ^ hashes[1]).count_ones() <= 8);
    assert!((hashes[0] ^ hashes[2]).count_ones() > 8);
}

#[test]
fn oversized_dimensions_do_not_abort_local_preparation() {
    let dir = common::tmp();
    catalogue(dir.path());
    // A PNG header claiming 9000 x 10 pixels: past the edge limit.
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = b"IHDR".to_vec();
    ihdr.extend_from_slice(&9000u32.to_be_bytes());
    ihdr.extend_from_slice(&10u32.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    png.extend_from_slice(&13u32.to_be_bytes());
    png.extend_from_slice(&ihdr);
    png.extend_from_slice(&crc32(&ihdr).to_be_bytes());
    for (kind, data) in [
        (
            &b"IDAT"[..],
            &[0x78u8, 0x9c, 0x63, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01][..],
        ),
        (&b"IEND"[..], &[][..]),
    ] {
        let mut chunk = kind.to_vec();
        chunk.extend_from_slice(data);
        png.extend_from_slice(&(data.len() as u32).to_be_bytes());
        png.extend_from_slice(&chunk);
        png.extend_from_slice(&crc32(&chunk).to_be_bytes());
    }
    write(&dir.path().join("thumbs/example/123.jpg"), &png);
    let result = prepare_covers(dir.path(), 1).unwrap();
    assert!(matches!(result["resized"], Value::Int(0)));
    assert!(
        result["errors"].as_array().unwrap()[0]
            .py_str()
            .contains("cover dimensions exceed limits")
    );
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

#[test]
fn doctor_counts_missing_descriptions_and_rejects_malformed_fields() {
    let dir = common::tmp();
    catalogue(dir.path());
    assert!(matches!(
        diagnose(dir.path())["without_description"],
        Value::Int(1)
    ));
    let mut shard = read_json(&dir.path().join("catalog/example.json"));
    {
        let album = shard
            .as_object_mut()
            .unwrap()
            .get_mut("albums")
            .unwrap()
            .as_array_mut()
            .unwrap()[0]
            .as_object_mut()
            .unwrap();
        album.insert("description".into(), "kept".into());
        album.insert("description_truncated".into(), true.into());
    }
    validate_metadata(&shard, "catalog/example.json").unwrap();
    for (field, bad) in [("description", "5"), ("description_truncated", r#""yes""#)] {
        let mut broken = shard.clone();
        broken
            .as_object_mut()
            .unwrap()
            .get_mut("albums")
            .unwrap()
            .as_array_mut()
            .unwrap()[0]
            .as_object_mut()
            .unwrap()
            .insert(field.into(), json(bad));
        assert!(
            validate_metadata(&broken, "catalog/example.json").is_err(),
            "{field}"
        );
    }
}

fn with_price(root: &Path, price: &str) -> Value {
    let mut shard = read_json(&root.join("catalog/example.json"));
    shard
        .as_object_mut()
        .unwrap()
        .get_mut("albums")
        .unwrap()
        .as_array_mut()
        .unwrap()[0]
        .as_object_mut()
        .unwrap()
        .insert("price".into(), json(price));
    shard
}

#[test]
fn metadata_price_rules() {
    let dir = common::tmp();
    catalogue(dir.path());
    let big = format!("1{}", "0".repeat(308));
    let max_int = format!("{:.0}", f64::MAX);
    let max_int_plus = {
        let mut digits: Vec<u8> = max_int.bytes().collect();
        let last = digits.len() - 1;
        digits[last] += 1;
        String::from_utf8(digits).unwrap()
    };
    for ok in [
        "0",
        "-0.0",
        "1",
        "12.5",
        big.as_str(),
        "1.7976931348623157e308",
        max_int.as_str(),
        max_int_plus.as_str(),
    ] {
        validate_metadata(&with_price(dir.path(), ok), "catalog/example.json")
            .unwrap_or_else(|e| panic!("{ok}: {e}"));
    }
    let huge = format!("1{}", "0".repeat(400));
    for bad in [
        "true",
        "false",
        "null",
        r#""1""#,
        "[]",
        "{}",
        "-1",
        "-0.1",
        "NaN",
        "Infinity",
        "-Infinity",
        huge.as_str(),
        &format!("-{huge}"),
    ] {
        let err = validate_metadata(&with_price(dir.path(), bad), "catalog/example.json")
            .unwrap_err()
            .to_string();
        assert!(err.contains("invalid price"), "{bad}: {err}");
    }
    let mut missing = read_json(&dir.path().join("catalog/example.json"));
    missing
        .as_object_mut()
        .unwrap()
        .get_mut("albums")
        .unwrap()
        .as_array_mut()
        .unwrap()[0]
        .as_object_mut()
        .unwrap()
        .shift_remove("price");
    validate_metadata(&missing, "catalog/example.json").unwrap();
}

#[test]
fn maintenance_reports_invalid_price_without_changing_metadata() {
    let huge = format!("1{}", "0".repeat(400));
    for command in ["doctor", "snapshot", "restore"] {
        for price in [huge.as_str(), "-1"] {
            let dir = common::tmp();
            catalogue(dir.path());
            let files = [
                dir.path().join("index.json"),
                dir.path().join("catalog/example.json"),
            ];
            let shard = with_price(dir.path(), price);
            let data = dir.path().to_string_lossy().into_owned();
            let mut args = vec!["--data".to_string(), data, command.to_string()];
            if command == "restore" {
                let archive = dir.path().join("bad.zip");
                make_zip(
                    &archive,
                    &[
                        ("index.json", std::fs::read(&files[0]).unwrap()),
                        (
                            "catalog/example.json",
                            pyjson::dumps(&shard, Style::Default).into_bytes(),
                        ),
                    ],
                );
                args.push(archive.to_string_lossy().into_owned());
            } else {
                // Written as Python's json.dumps would, so large integers stay integers.
                write(&files[1], pyjson::dumps(&shard, Style::Compact));
            }
            let before: Vec<Vec<u8>> = files.iter().map(|p| std::fs::read(p).unwrap()).collect();
            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
            let (code, out, err) = lamp(&refs, None);
            assert_eq!(code, 1, "{command} {price}");
            assert!(format!("{out}{err}").contains("invalid price"));
            if command == "doctor" {
                assert!(!obj(&json(&out))["issues"].as_array().unwrap().is_empty());
            }
            let after: Vec<Vec<u8>> = files.iter().map(|p| std::fs::read(p).unwrap()).collect();
            assert_eq!(before, after);
            assert!(!dir.path().join("snapshots").exists());
        }
    }
}

#[test]
fn load_shard_reports_oversized_price_as_corrupt() {
    let dir = common::tmp();
    catalogue(dir.path());
    let shard = with_price(dir.path(), &format!("1{}", "0".repeat(400)));
    write(
        &dir.path().join("catalog/example.json"),
        pyjson::dumps(&shard, Style::Compact),
    );
    let err = load_shard(dir.path(), "example").unwrap_err().to_string();
    assert!(
        err.starts_with("corrupt shard") && err.contains("invalid price"),
        "{err}"
    );
}

fn several_sellers(root: &Path) {
    let sellers = ["first", "second", "third"];
    let list: Vec<String> = sellers
        .iter()
        .map(|s| format!(r#"{{"id": "{s}", "shard": "catalog/{s}.json"}}"#))
        .collect();
    atomic_write_json(
        &root.join("index.json"),
        &json(&format!(r#"{{"sellers": [{}]}}"#, list.join(","))),
    )
    .unwrap();
    for (position, seller) in sellers.iter().enumerate() {
        let mut albums = Vec::new();
        for number in 0..2 {
            let id = position * 10 + number + 1;
            let thumb = format!("thumbs/{seller}/{id}.png");
            write(&root.join(&thumb), common::png(8, 8, [255, 0, 0]));
            albums.push(format!(r#"{{"id": "{id}", "seller": "{seller}", "title": "Synthetic", "album_url": "https://{seller}.x.yupoo.com/albums/{id}", "links": [], "thumb": "{thumb}"}}"#));
        }
        atomic_write_json(
            &root.join(format!("catalog/{seller}.json")),
            &json(&format!(
                r#"{{"seller": "{seller}", "albums": [{}]}}"#,
                albums.join(",")
            )),
        )
        .unwrap();
    }
}

fn prepared(root: &Path, seller: &str) -> Vec<String> {
    let shard = read_json(&root.join(format!("catalog/{seller}.json")));
    obj(&shard)["albums"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| {
            obj(a)
                .get("cover_prepared")
                .map_or("0".into(), Value::py_str)
        })
        .collect()
}

#[test]
fn cover_limit_leaves_later_shards_untouched_after_validating_the_snapshot() {
    for (limit, rewritten) in [
        (0, [false, false, false]),
        (1, [true, false, false]),
        (2, [true, false, false]),
        (3, [true, true, false]),
        (9, [true, true, true]),
    ] {
        let dir = common::tmp();
        let root = dir.path();
        several_sellers(root);
        let names = [
            "index.json",
            "catalog/first.json",
            "catalog/second.json",
            "catalog/third.json",
        ];
        let before: Vec<Vec<u8>> = names
            .iter()
            .map(|n| std::fs::read(root.join(n)).unwrap())
            .collect();
        let result = prepare_covers(root, limit).unwrap();
        assert!(pyjson::py_eq(
            &result["processed"],
            &Value::Int(limit.min(6))
        ));
        assert_eq!(result["errors"].py_str(), "[]");
        let snapshot = PathBuf::from(result["snapshot"].py_str());
        for (name, bytes) in names.iter().zip(&before) {
            assert_eq!(&zip_member(&snapshot, name), bytes);
        }
        let all: Vec<String> = ["first", "second", "third"]
            .iter()
            .flat_map(|s| prepared(root, s))
            .collect();
        let expected: Vec<String> = (0..6)
            .map(|i| if i < limit { "1".into() } else { "0".into() })
            .collect();
        assert_eq!(all, expected, "limit {limit}");
        assert_eq!(std::fs::read(root.join("index.json")).unwrap(), before[0]);
        for (i, changed) in rewritten.iter().enumerate() {
            if !changed {
                assert_eq!(
                    std::fs::read(root.join(names[i + 1])).unwrap(),
                    before[i + 1]
                );
            }
        }
    }
}

#[test]
fn cover_batches_resume_across_sellers() {
    let dir = common::tmp();
    several_sellers(dir.path());
    for expected in [["1", "0"], ["1", "1"]] {
        assert!(matches!(
            prepare_covers(dir.path(), 1).unwrap()["processed"],
            Value::Int(1)
        ));
        assert_eq!(prepared(dir.path(), "first"), expected);
    }
    assert!(matches!(
        prepare_covers(dir.path(), 1).unwrap()["processed"],
        Value::Int(1)
    ));
    assert_eq!(prepared(dir.path(), "second"), ["1", "0"]);
}

#[test]
fn failed_cover_attempt_consumes_the_batch_and_remains_retryable() {
    let dir = common::tmp();
    several_sellers(dir.path());
    std::fs::remove_file(dir.path().join("thumbs/first/1.png")).unwrap();
    let before: Vec<Vec<u8>> = ["first", "second", "third"]
        .iter()
        .map(|s| std::fs::read(dir.path().join(format!("catalog/{s}.json"))).unwrap())
        .collect();
    for _ in 0..2 {
        let result = prepare_covers(dir.path(), 1).unwrap();
        assert!(matches!(result["processed"], Value::Int(1)));
        assert_eq!(result["errors"].as_array().unwrap().len(), 1);
        let after: Vec<Vec<u8>> = ["first", "second", "third"]
            .iter()
            .map(|s| std::fs::read(dir.path().join(format!("catalog/{s}.json"))).unwrap())
            .collect();
        assert_eq!(before, after);
    }
}

#[test]
fn cover_limit_preserves_validation_of_later_shards() {
    let dir = common::tmp();
    several_sellers(dir.path());
    write(&dir.path().join("catalog/third.json"), "invalid");
    let before = tree(dir.path());
    assert!(prepare_covers(dir.path(), 1).is_err());
    for (path, bytes) in &before {
        assert!(
            std::fs::read(path).unwrap() == *bytes,
            "{} changed",
            path.display()
        );
    }
    assert!(!dir.path().join("snapshots").exists());
}
