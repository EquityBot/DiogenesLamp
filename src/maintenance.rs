//! Read-only diagnosis and explicit local metadata recovery.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use crate::covers::{cover_facts, resize_cover};
use crate::errors::{Error, Result, value};
use crate::locking::CatalogueLock;
use crate::pyerr::{self, Call};
use crate::pyjson::{self, Map, Value};
use crate::schema::{canonical_album, data_path, validate_metadata};
use crate::storage::atomic_write_json;

pub const MAX_METADATA_BYTES: u64 = 256 * 1024 * 1024;
pub const MAX_METADATA_FILES: usize = 10000;

fn stat_len(path: &Path) -> Result<u64> {
    std::fs::metadata(path)
        .map(|m| m.len())
        .map_err(|e| Error::Os(pyerr::os_error(&e, Call::Os, path)))
}

pub fn read_metadata(path: &Path, relative: &str) -> Result<Value> {
    if stat_len(path)? > MAX_METADATA_BYTES {
        return value("metadata file exceeds 256 MB");
    }
    let text = pyerr::read_text(path).map_err(|f| Error::Os(f.message))?;
    let parsed = pyjson::loads(&text).map_err(|e| Error::Value(e.0))?;
    validate_metadata(&parsed, relative)?;
    Ok(parsed)
}

/// `sorted(dir.glob("*.json"))`: case-insensitive matching and ordering on
/// Windows, as pathlib does there.
fn json_files(dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| {
            if cfg!(windows) {
                name.to_lowercase().ends_with(".json")
            } else {
                name.ends_with(".json")
            }
        })
        .collect();
    if cfg!(windows) {
        names.sort_by_key(|n| n.to_lowercase());
    } else {
        names.sort();
    }
    names
        .into_iter()
        .map(|n| (n.clone(), dir.join(n)))
        .collect()
}

/// Metadata files as (relative posix path, path).
pub fn metadata_files(root: &Path) -> Result<Vec<(String, PathBuf)>> {
    let mut files = Vec::new();
    let index = root.join("index.json");
    if index.exists() {
        files.push(("index.json".to_string(), index));
    }
    for dir in ["catalog", "runs"] {
        for (name, path) in json_files(&root.join(dir)) {
            files.push((format!("{dir}/{name}"), path));
        }
    }
    if files.len() > MAX_METADATA_FILES {
        return value("too many metadata files");
    }
    for (relative, _) in &files {
        data_path(root, relative, false)?;
    }
    let mut total = 0u64;
    for (_, path) in &files {
        total += stat_len(path)?;
    }
    if total > MAX_METADATA_BYTES {
        return value("metadata exceeds 256 MB");
    }
    Ok(files)
}

fn sellers_of(index: &Value) -> Vec<Map> {
    index
        .as_object()
        .and_then(|m| m.get("sellers"))
        .and_then(Value::as_array)
        .map(|s| s.iter().filter_map(|v| v.as_object().cloned()).collect())
        .unwrap_or_default()
}

fn albums_of(shard: &Value) -> Vec<Map> {
    shard
        .as_object()
        .and_then(|m| m.get("albums"))
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|v| v.as_object().cloned()).collect())
        .unwrap_or_default()
}

fn id_of(album: &Map) -> String {
    album.get("id").map(Value::py_str).unwrap_or_default()
}

fn truthy(map: &Map, key: &str) -> bool {
    map.get(key).is_some_and(Value::truthy)
}

pub fn diagnose(root: &Path) -> Map {
    let mut counts: Vec<(&str, i128)> = vec![
        ("albums", 0),
        ("unadmitted", 0),
        ("missing_covers", 0),
        ("unpriced", 0),
        ("without_detail_date", 0),
        ("without_description", 0),
        ("gone", 0),
        ("detail_failures", 0),
    ];
    let mut issues: Vec<String> = Vec::new();
    let add = |counts: &mut Vec<(&str, i128)>, key: &str, n: i128| {
        if let Some(entry) = counts.iter_mut().find(|(k, _)| *k == key) {
            entry.1 += n;
        }
    };
    let index = data_path(root, "index.json", false).and_then(|p| read_metadata(&p, "index.json"));
    match index {
        Err(e) => issues.push(format!("index.json: {e}")),
        Ok(index) => {
            let selected_index = matches!(
                index.as_object().and_then(|m| m.get("selection_version")),
                Some(Value::Int(1))
            );
            let mut ids: std::collections::HashSet<String> = Default::default();
            for seller in sellers_of(&index) {
                let relative = seller.get("shard").map(Value::py_str).unwrap_or_default();
                let outcome = (|| -> Result<()> {
                    let shard = read_metadata(&data_path(root, &relative, false)?, &relative)?;
                    let albums = albums_of(&shard);
                    let selected: std::collections::HashSet<String> = match seller.get("album_ids")
                    {
                        Some(Value::Array(items)) => items.iter().map(Value::py_str).collect(),
                        _ => albums.iter().map(id_of).collect(),
                    };
                    let present: std::collections::HashSet<String> =
                        albums.iter().map(id_of).collect();
                    if !selected.is_subset(&present) {
                        issues.push(format!("{relative}: indexed album missing from shard"));
                    }
                    let count = albums
                        .iter()
                        .filter(|a| selected.contains(&id_of(a)) && !truthy(a, "gone_since"))
                        .count();
                    if let Some(listed) = seller.get("album_count")
                        && !pyjson::py_eq(listed, &Value::from(count))
                    {
                        issues.push(format!("{relative}: index count differs from shard"));
                    }
                    for album in &albums {
                        add(&mut counts, "albums", 1);
                        let admitted = canonical_album(album);
                        add(&mut counts, "unadmitted", i128::from(!admitted));
                        let id = id_of(album);
                        if selected_index && selected.contains(&id) && !admitted {
                            issues
                                .push(format!("{relative}: selected album is inadmissible: {id}"));
                        }
                        if ids.contains(&id) {
                            issues.push(format!(
                                "{relative}: duplicate album ID across shards: {id}"
                            ));
                        }
                        ids.insert(id);
                        add(&mut counts, "unpriced", i128::from(!truthy(album, "price")));
                        add(&mut counts, "gone", i128::from(truthy(album, "gone_since")));
                        add(
                            &mut counts,
                            "without_detail_date",
                            i128::from(!truthy(album, "updated_at")),
                        );
                        add(
                            &mut counts,
                            "without_description",
                            i128::from(!album.contains_key("description")),
                        );
                        if let Some(Value::Str(thumb)) = album.get("thumb")
                            && !thumb.is_empty()
                            && !data_path(root, thumb, true)?.is_file()
                        {
                            add(&mut counts, "missing_covers", 1);
                        }
                    }
                    let run_relative = format!(
                        "runs/{}.json",
                        seller.get("id").map(Value::py_str).unwrap_or_default()
                    );
                    let run_path = data_path(root, &run_relative, false)?;
                    if run_path.exists() {
                        let run = read_metadata(&run_path, &run_relative)?;
                        let failed = run
                            .as_object()
                            .and_then(|m| m.get("failed_ids"))
                            .and_then(Value::as_array)
                            .map_or(0, Vec::len);
                        add(&mut counts, "detail_failures", failed as i128);
                    }
                    Ok(())
                })();
                if let Err(e) = outcome {
                    issues.push(format!("{relative}: {e}"));
                }
            }
        }
    }
    let mut report = crate::obj! { "schema_version" => 1i64 };
    for (key, n) in counts {
        report.insert(key.into(), Value::Int(n));
    }
    report.insert(
        "issues".into(),
        Value::Array(issues.into_iter().map(Value::Str).collect()),
    );
    report
}

fn random_hex(n: usize) -> String {
    let mut bytes = vec![0u8; n.div_ceil(2)];
    let _ = getrandom::fill(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()[..n].to_string()
}

fn zip_time(path: &Path) -> zip::DateTime {
    use chrono::{Datelike, Timelike};
    let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    let local: chrono::DateTime<chrono::Local> =
        modified.map_or_else(chrono::Local::now, chrono::DateTime::from);
    zip::DateTime::from_date_and_time(
        local.year().clamp(1980, 2107) as u16,
        local.month() as u8,
        local.day() as u8,
        local.hour() as u8,
        local.minute() as u8,
        local.second() as u8,
    )
    .unwrap_or_default()
}

fn snapshot_inner(root: &Path, validate: bool) -> Result<PathBuf> {
    let files = metadata_files(root)?;
    if validate {
        let index = read_metadata(&data_path(root, "index.json", false)?, "index.json")?;
        for seller in sellers_of(&index) {
            let shard = seller.get("shard").map(Value::py_str).unwrap_or_default();
            if !data_path(root, &shard, false)?.is_file() {
                return value(format!("{shard}: missing indexed shard"));
            }
        }
        for (relative, path) in &files {
            read_metadata(path, relative)?;
        }
    }
    let directory = root.join("snapshots");
    if let Err(e) = std::fs::create_dir(&directory)
        && !(e.kind() == std::io::ErrorKind::AlreadyExists && directory.is_dir())
    {
        return Err(Error::Os(pyerr::os_error(&e, Call::Os, &directory)));
    }
    if !pyerr::resolve(&directory).starts_with(pyerr::resolve(root)) {
        return value("snapshot directory escapes data directory");
    }
    let name = format!(
        "{}-{}.zip",
        chrono::Utc::now().format("%Y%m%dT%H%M%S"),
        random_hex(8)
    );
    let path = directory.join(name);
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|e| Error::Os(pyerr::os_error(&e, Call::Open, &path)))?;
    let written = (|| -> Result<()> {
        let mut archive = zip::ZipWriter::new(file);
        for (relative, source) in &files {
            let data = pyerr::read_bytes(source).map_err(|f| Error::Os(f.message))?;
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated)
                .last_modified_time(zip_time(source));
            archive
                .start_file(relative.as_str(), options)
                .map_err(|e| Error::Os(e.to_string()))?;
            archive
                .write_all(&data)
                .map_err(|e| Error::Os(e.to_string()))?;
        }
        archive.finish().map_err(|e| Error::Os(e.to_string()))?;
        Ok(())
    })();
    if let Err(e) = written {
        let _ = std::fs::remove_file(&path);
        return Err(e);
    }
    Ok(path)
}

/// Save a bounded metadata snapshot (covers excluded).
pub fn snapshot(root: &Path) -> Result<PathBuf> {
    let _lock = CatalogueLock::acquire(root)?;
    snapshot_inner(root, true)
}

/// Restore metadata from a snapshot; returns where the previous files went.
pub fn restore(root: &Path, archive_path: &Path) -> Result<Option<PathBuf>> {
    let _lock = CatalogueLock::acquire(root)?;
    let file = std::fs::File::open(archive_path)
        .map_err(|e| Error::Os(pyerr::os_error(&e, Call::Open, archive_path)))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|_| Error::BadZip("File is not a zip file".into()))?;
    let mut total = 0u64;
    let mut names = Vec::new();
    for i in 0..archive.len() {
        let entry = archive
            .by_index_raw(i)
            .map_err(|_| Error::BadZip("File is not a zip file".into()))?;
        total += entry.size();
        names.push(entry.name().map(|n| n.into_owned()).unwrap_or_default());
    }
    if names.len() > MAX_METADATA_FILES || total > MAX_METADATA_BYTES {
        return value("snapshot exceeds recovery limits");
    }
    let mut records: Vec<(String, Value)> = Vec::new();
    for (i, name) in names.iter().enumerate() {
        data_path(root, name, false)?;
        if records.iter().any(|(n, _)| n == name) {
            return value("duplicate snapshot member");
        }
        let mut entry = archive
            .by_index(i)
            .map_err(|e| Error::BadZip(e.to_string()))?;
        let mut data = Vec::new();
        entry.read_to_end(&mut data).map_err(|_| {
            Error::BadZip(format!("Bad CRC-32 for file {}", crate::pystr::repr(name)))
        })?;
        let parsed = pyjson::loads_bytes(&data).map_err(Error::Value)?;
        validate_metadata(&parsed, name)?;
        records.push((name.clone(), parsed));
    }
    let Some((_, index)) = records.iter().find(|(n, _)| n == "index.json") else {
        return value("snapshot has no index");
    };
    for seller in sellers_of(index) {
        let shard = seller.get("shard").map(Value::py_str).unwrap_or_default();
        if !records.iter().any(|(n, _)| *n == shard) {
            return value("snapshot is missing an indexed shard");
        }
    }
    let previous = if metadata_files(root)?.is_empty() {
        None
    } else {
        Some(snapshot_inner(root, false)?)
    };
    for (relative, payload) in &records {
        if relative != "index.json" {
            atomic_write_json(&data_path(root, relative, false)?, payload)?;
        }
    }
    let index = records
        .iter()
        .find(|(n, _)| n == "index.json")
        .map(|(_, v)| v.clone())
        .unwrap_or(Value::Null);
    atomic_write_json(&data_path(root, "index.json", false)?, &index)?;
    Ok(previous)
}

/// Resize and fingerprint cached covers, at most `limit` per run.
pub fn prepare_covers(root: &Path, limit: i128) -> Result<Map> {
    let mut processed: i128 = 0;
    let mut resized: i128 = 0;
    let mut before_bytes: i128 = 0;
    let mut after_bytes: i128 = 0;
    let mut errors: Vec<String> = Vec::new();
    let _lock = CatalogueLock::acquire(root)?;
    let snapshot_path = snapshot_inner(root, true)?;
    let index = read_metadata(&data_path(root, "index.json", false)?, "index.json")?;
    for seller in sellers_of(&index) {
        if processed >= limit {
            break;
        }
        let relative = seller.get("shard").map(Value::py_str).unwrap_or_default();
        let mut shard = read_metadata(&data_path(root, &relative, false)?, &relative)?;
        let mut changed = false;
        if let Some(Value::Array(albums)) = shard.as_object_mut().and_then(|m| m.get_mut("albums"))
        {
            for album in albums.iter_mut() {
                if processed >= limit {
                    break;
                }
                let Some(album) = album.as_object_mut() else {
                    continue;
                };
                let thumb = match album.get("thumb") {
                    Some(Value::Str(t)) if !t.is_empty() => t.clone(),
                    _ => continue,
                };
                let path = data_path(root, &thumb, true)?;
                let prepared = album
                    .get("cover_prepared")
                    .is_some_and(|v| pyjson::py_eq(v, &Value::Int(1)));
                if prepared && path.is_file() {
                    continue;
                }
                processed += 1;
                let attempt = (|| -> Result<()> {
                    let size = stat_len(&path)?;
                    if size > 8 * 1024 * 1024 {
                        return value("cover exceeds 8 MB");
                    }
                    before_bytes += i128::from(size);
                    resized += i128::from(resize_cover(&path)?);
                    after_bytes += i128::from(stat_len(&path)?);
                    for (k, v) in cover_facts(&path) {
                        album.insert(k, v);
                    }
                    album.insert("cover_prepared".into(), Value::Int(1));
                    Ok(())
                })();
                match attempt {
                    Ok(()) => changed = true,
                    Err(e) => errors.push(format!("{}: {e}", id_of(album))),
                }
            }
        }
        if changed {
            if let Some(map) = shard.as_object_mut() {
                map.insert("schema_version".into(), Value::Int(1));
            }
            atomic_write_json(&data_path(root, &relative, false)?, &shard)?;
        }
    }
    let mut result = crate::obj! {
        "processed" => Value::Int(processed),
        "resized" => Value::Int(resized),
        "before_bytes" => Value::Int(before_bytes),
        "after_bytes" => Value::Int(after_bytes),
    };
    result.insert(
        "errors".into(),
        Value::Array(errors.into_iter().map(Value::Str).collect()),
    );
    result.insert("snapshot".into(), pyerr::path_str(&snapshot_path).into());
    Ok(result)
}
