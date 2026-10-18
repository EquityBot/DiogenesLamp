//! Catalogue storage: per-seller shards plus an index manifest, all atomic
//! writes.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use indexmap::IndexMap;

use crate::dedup::newest_listings;
use crate::errors::{Error, Result, value};
use crate::locking::CatalogueLock;
use crate::pyerr::{self, Call};
use crate::pyjson::{self, Map, Style, Value};
use crate::schema::{data_path, validate_metadata};

/// Shards keyed by album id, in file order.
pub type Shard = IndexMap<String, Map>;

const TEMP_CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789_";

/// A fresh name in `dir` like `tempfile.NamedTemporaryFile(prefix, suffix)`.
pub fn temp_name(prefix: &str, suffix: &str) -> String {
    let mut bytes = [0u8; 8];
    let _ = getrandom::fill(&mut bytes);
    let middle: String = bytes
        .iter()
        .map(|b| TEMP_CHARS[usize::from(*b) % TEMP_CHARS.len()] as char)
        .collect();
    format!("{prefix}{middle}{suffix}")
}

/// Create a new temporary file beside `dest`, as NamedTemporaryFile does.
pub fn create_temp(dir: &Path, prefix: &str, suffix: &str) -> Result<(PathBuf, std::fs::File)> {
    for _ in 0..100 {
        let path = dir.join(temp_name(prefix, suffix));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(Error::Os(pyerr::os_error(&e, Call::Open, &path))),
        }
    }
    Err(Error::Os("No usable temporary file name found".into()))
}

/// Write JSON through a temporary file and a replace, so readers never see
/// a partial file. The replace is retried: on Windows it can lose briefly
/// against a reader holding the destination open.
pub fn atomic_write_json(path: &Path, obj: &Value) -> Result<()> {
    let body = pyerr::native_newlines(&pyjson::dumps(obj, Style::Compact));
    let parent = path.parent().unwrap_or(Path::new("."));
    pyerr::mkdir_parents(parent).map_err(Error::Os)?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let (tmp, mut file) = create_temp(parent, &format!("{name}."), ".tmp")?;
    let result = (|| {
        file.write_all(body.as_bytes())
            .map_err(|e| Error::Os(pyerr::os_error(&e, Call::Open, &tmp)))?;
        drop(file);
        for attempt in 0..5 {
            match std::fs::rename(&tmp, path) {
                Ok(()) => return Ok(()),
                Err(e) if attempt == 4 => {
                    return Err(Error::Os(pyerr::os_error2(&e, Call::Os, &tmp, path)));
                }
                Err(_) => std::thread::sleep(Duration::from_secs_f64(0.1 * 2f64.powi(attempt))),
            }
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

fn shard_path(data_dir: &Path, seller: &str) -> Result<PathBuf> {
    data_path(data_dir, &format!("catalog/{seller}.json"), false)
}

/// A seller's shard keyed by album id; missing means empty. A corrupt or
/// unreadable shard is a ValueError naming the file.
pub fn load_shard(data_dir: &Path, seller: &str) -> Result<Shard> {
    let path = shard_path(data_dir, seller)?;
    if !path.exists() {
        return Ok(Shard::new());
    }
    let corrupt =
        |e: String| Error::Value(format!("corrupt shard {}: {e}", pyerr::path_str(&path)));
    let text = pyerr::read_text(&path).map_err(|f| corrupt(f.message))?;
    let payload = pyjson::loads(&text).map_err(|e| corrupt(e.0))?;
    validate_metadata(&payload, &format!("catalog/{seller}.json"))
        .map_err(|e| corrupt(e.to_string()))?;
    let mut shard = Shard::new();
    if let Some(Value::Array(albums)) = payload.as_object().and_then(|p| p.get("albums")) {
        for album in albums {
            if let Some(map) = album.as_object() {
                let id = map
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                shard.insert(id, map.clone());
            }
        }
    }
    Ok(shard)
}

fn int_order(id: &str) -> (usize, String) {
    let trimmed = id.trim_start_matches('0');
    (trimmed.len(), trimmed.to_string())
}

/// Write a seller's shard, newest (highest) album id first.
pub fn write_shard(data_dir: &Path, seller: &str, albums: &Shard) -> Result<()> {
    let mut ordered: Vec<&Map> = albums.values().collect();
    let key = |a: &Map| int_order(&a.get("id").map(Value::py_str).unwrap_or_default());
    ordered.sort_by_key(|a| std::cmp::Reverse(key(a)));
    let payload = crate::obj! {
        "schema_version" => 1i64,
        "seller" => seller,
        "albums" => ordered.into_iter().map(|a| Value::Object(a.clone())).collect::<Vec<_>>(),
    };
    atomic_write_json(&shard_path(data_dir, seller)?, &Value::Object(payload))
}

/// Facts about the album itself: once captured they stay captured.
const STICKY_FIELDS: [&str; 2] = ["date_published", "date_modified"];

pub fn record_change(album: &mut Map, field: &str, before: Value, after: Value, run_ts: &str) {
    if pyjson::py_eq(&before, &after) {
        return;
    }
    let mut history = match album.get("history") {
        Some(Value::Array(items)) => items.clone(),
        _ => Vec::new(),
    };
    history.push(Value::Object(crate::obj! {
        "at" => run_ts,
        "field" => field,
        "before" => before,
        "after" => after,
    }));
    let start = history.len().saturating_sub(50);
    album.insert("history".into(), Value::Array(history.split_off(start)));
}

fn truthy(map: &Map, key: &str) -> bool {
    map.get(key).is_some_and(Value::truthy)
}

/// Merge a freshly scraped record with the prior one, keeping first_seen.
pub fn merge_album(old: Option<&Map>, new: &Map, run_ts: &str) -> Map {
    let mut merged = new.clone();
    let old = old.filter(|o| !o.is_empty());
    if let Some(old) = old {
        let history = match old.get("history") {
            Some(Value::Array(items)) => items.clone(),
            _ => Vec::new(),
        };
        merged.insert("history".into(), Value::Array(history));
        if truthy(old, "gone_since") {
            merged.insert("gone_since".into(), old["gone_since"].clone());
        }
        for key in ["thumb_version", "cover_hash", "cover_prepared"] {
            if !merged.contains_key(key)
                && let Some(v) = old.get(key)
            {
                merged.insert(key.into(), v.clone());
            }
        }
        let old_price = truthy(old, "price");
        let new_price = truthy(new, "price");
        if old_price && new_price {
            record_change(
                &mut merged,
                "price",
                old["price"].clone(),
                new["price"].clone(),
                run_ts,
            );
        } else if old_price != new_price {
            let label = |known: bool| if known { "known" } else { "unknown" };
            record_change(
                &mut merged,
                "price availability",
                label(old_price).into(),
                label(new_price).into(),
                run_ts,
            );
        }
        let links = |m: &Map| m.get("links").cloned().unwrap_or(Value::Array(Vec::new()));
        record_change(
            &mut merged,
            "purchase links",
            links(old),
            links(new),
            run_ts,
        );
    }
    let first_seen = match old {
        Some(old) => old
            .get("first_seen")
            .cloned()
            .unwrap_or_else(|| run_ts.into()),
        None => run_ts.into(),
    };
    merged.insert("first_seen".into(), first_seen);
    merged.insert("updated_at".into(), run_ts.into());
    if let Some(old) = old {
        for field in STICKY_FIELDS {
            if !truthy(&merged, field) && truthy(old, field) {
                merged.insert(field.into(), old[field].clone());
            }
        }
    }
    merged
}

fn seller_id_of(seller: &Map) -> String {
    seller.get("id").map(Value::py_str).unwrap_or_default()
}

/// Index only the newest duplicate listing; retain captures for refresh and
/// recovery.
pub fn write_index(
    data_dir: &Path,
    sellers: &[Map],
    shops: &Value,
    run_ts: &str,
    strict: bool,
) -> Result<()> {
    let mut captures: Vec<Map> = Vec::new();
    let mut readable: Vec<String> = Vec::new();
    for seller in sellers {
        let id = seller_id_of(seller);
        let attempt = (|| -> Result<Shard> {
            if !shard_path(data_dir, &id)?.is_file() {
                return value(format!("missing shard for {id}"));
            }
            load_shard(data_dir, &id)
        })();
        match attempt {
            Ok(shard) => {
                captures.extend(shard.into_values());
                readable.push(id);
            }
            Err(Error::Value(message)) => {
                if strict {
                    return Err(Error::Value(message));
                }
            }
            Err(other) => return Err(other),
        }
    }
    let mut ids: Vec<String> = captures
        .iter()
        .map(|a| a.get("id").map(Value::py_str).unwrap_or_default())
        .collect();
    let total = ids.len();
    ids.sort();
    ids.dedup();
    if ids.len() != total {
        return value("duplicate album ID across shards");
    }
    let (selected, aliases) = newest_listings(&captures);
    let mut indexed = Vec::new();
    for seller in sellers {
        let id = seller_id_of(seller);
        if !readable.contains(&id) {
            indexed.push(Value::Object(seller.clone()));
            continue;
        }
        let albums: Vec<&Map> = selected
            .iter()
            .filter(|a| a.get("seller").map(Value::py_str).as_deref() == Some(id.as_str()))
            .collect();
        let mut entry = seller.clone();
        entry.insert(
            "album_ids".into(),
            Value::Array(
                albums
                    .iter()
                    .map(|a| a.get("id").cloned().unwrap_or(Value::Null))
                    .collect(),
            ),
        );
        let count = albums.iter().filter(|a| !truthy(a, "gone_since")).count();
        entry.insert("album_count".into(), count.into());
        indexed.push(Value::Object(entry));
    }
    let selected_ids: std::collections::HashSet<String> = selected
        .iter()
        .map(|a| a.get("id").map(Value::py_str).unwrap_or_default())
        .collect();
    let mut alias_map = Map::new();
    for (old, new) in aliases {
        if !selected_ids.contains(&old) {
            alias_map.insert(old, new.into());
        }
    }
    let mut index = crate::obj! { "schema_version" => 1i64 };
    if readable.len() == sellers.len() {
        index.insert("selection_version".into(), 1i64.into());
    }
    index.insert("generated_at".into(), run_ts.into());
    index.insert("sellers".into(), Value::Array(indexed));
    index.insert("shops".into(), shops.clone());
    index.insert("album_aliases".into(), Value::Object(alias_map));
    atomic_write_json(&data_dir.join("index.json"), &Value::Object(index))
}

fn read_json(path: &Path) -> Result<Value> {
    let text = pyerr::read_text(path).map_err(|f| Error::Os(f.message))?;
    pyjson::loads(&text).map_err(|e| Error::Value(e.0))
}

/// Rebuild the manifest from local captures, without fetching or rewriting
/// shards. Returns (indexed, duplicates).
pub fn reindex(data_dir: &Path) -> Result<(i128, usize)> {
    let _lock = CatalogueLock::acquire(data_dir)?;
    let path = data_path(data_dir, "index.json", false)?;
    let index = read_json(&path)?;
    validate_metadata(&index, "index.json")?;
    let map = index.as_object().cloned().unwrap_or_default();
    let Some(Value::Str(generated)) = map.get("generated_at") else {
        return value("index generated_at must be text");
    };
    let sellers: Vec<Map> = map
        .get("sellers")
        .and_then(Value::as_array)
        .map(|s| s.iter().filter_map(|v| v.as_object().cloned()).collect())
        .unwrap_or_default();
    let shops = map
        .get("shops")
        .cloned()
        .unwrap_or(Value::Array(Vec::new()));
    write_index(data_dir, &sellers, &shops, generated, true)?;
    let rebuilt = read_json(&path)?;
    let rebuilt = rebuilt.as_object().cloned().unwrap_or_default();
    let indexed = rebuilt
        .get("sellers")
        .and_then(Value::as_array)
        .map(|s| {
            s.iter()
                .map(
                    |seller| match seller.as_object().and_then(|m| m.get("album_count")) {
                        Some(Value::Int(n)) => *n,
                        _ => 0,
                    },
                )
                .sum()
        })
        .unwrap_or(0);
    let duplicates = rebuilt
        .get("album_aliases")
        .and_then(Value::as_object)
        .map_or(0, Map::len);
    Ok((indexed, duplicates))
}

pub fn read_run(data_dir: &Path, seller: &str) -> Result<Map> {
    let relative = format!("runs/{seller}.json");
    let path = data_path(data_dir, &relative, false)?;
    let unreadable = |e: String| {
        Error::Value(format!(
            "unreadable run summary {}: {e}",
            pyerr::path_str(&path)
        ))
    };
    let text = match pyerr::read_text(&path) {
        Ok(text) => text,
        Err(fail) if fail.not_found => return Ok(Map::new()),
        Err(fail) => return Err(unreadable(fail.message)),
    };
    let value_read = pyjson::loads(&text).map_err(|e| unreadable(e.0))?;
    validate_metadata(&value_read, &relative).map_err(|e| unreadable(e.to_string()))?;
    Ok(value_read.as_object().cloned().unwrap_or_default())
}

fn string_list(map: &Map, key: &str) -> Vec<String> {
    match map.get(key) {
        Some(Value::Array(items)) => items.iter().map(Value::py_str).collect(),
        _ => Vec::new(),
    }
}

pub fn write_run(data_dir: &Path, seller: &str, run_ts: &str, mut summary: Map) -> Result<()> {
    let previous = read_run(data_dir, seller)?;
    summary.insert("schema_version".into(), 1i64.into());
    let resolved = string_list(&summary, "resolved_ids");
    summary.shift_remove("resolved_ids");
    let mut failed: std::collections::BTreeSet<String> = string_list(&previous, "failed_ids")
        .into_iter()
        .filter(|id| !resolved.contains(id))
        .collect();
    failed.extend(string_list(&summary, "failed_ids"));
    summary.insert(
        "failed_ids".into(),
        Value::Array(failed.iter().map(|s| Value::Str(s.clone())).collect()),
    );
    summary.insert("last_attempt".into(), run_ts.into());
    let last_success = previous
        .get("last_success")
        .cloned()
        .unwrap_or_else(|| "".into());
    summary.insert("last_success".into(), last_success);
    let complete = truthy(&summary, "listing_complete");
    let last_listing = if complete {
        run_ts.into()
    } else {
        previous
            .get("last_listing")
            .cloned()
            .unwrap_or_else(|| "".into())
    };
    summary.insert("last_listing".into(), last_listing);
    if complete && !truthy(&summary, "error") && failed.is_empty() {
        summary.insert("last_success".into(), run_ts.into());
    }
    atomic_write_json(
        &data_path(data_dir, &format!("runs/{seller}.json"), false)?,
        &Value::Object(summary),
    )
}
