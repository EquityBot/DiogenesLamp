//! Select the newest listing using the viewer's duplicate identities.

use std::collections::{BTreeSet, HashMap};

use unicode_normalization::UnicodeNormalization;

use crate::age::estimate_published;
use crate::isodate::{self, Date};
use crate::links::SHORTLINK_HOSTS;
use crate::pyjson::{Map, Value};
use crate::pystr;
use crate::pyurl;
use crate::schema::{canonical_album, public_https_host};

fn item_key(link: &Value) -> Option<String> {
    let url_text = link.as_object()?.get("url")?.as_str()?;
    let url = pyurl::urlsplit(&url_text.replace('\\', "/")).ok()?;
    let host = public_https_host(url_text).ok()?;
    // URL() removes dot segments, including encoded dots, before keying paths.
    let parts: Vec<String> = url
        .path
        .split('/')
        .map(|p| {
            let unquoted = pyurl::unquote(p);
            if unquoted == "." || unquoted == ".." {
                unquoted
            } else {
                p.to_string()
            }
        })
        .collect();
    let joined = pyurl::urljoin(
        "https://example.invalid/",
        &format!("/.{}", parts.join("/")),
    )
    .ok()?;
    let path = pyurl::urlsplit(&joined).ok()?.path;
    let path = pyurl::quote(&path, "/!$&'()*+,;=:@%[]-._~|");
    let platform = ["taobao", "tmall", "weidian", "1688"]
        .into_iter()
        .find(|p| host == format!("{p}.com") || host.ends_with(&format!(".{p}.com")));
    let mut ids: BTreeSet<String> = pyurl::parse_qsl(url.query_str())
        .into_iter()
        .filter(|(k, v)| matches!(k.to_lowercase().as_str(), "id" | "itemid") && !v.is_empty())
        .map(|(_, v)| v)
        .collect();
    if platform == Some("1688")
        && let Some(id) = path
            .strip_prefix("/offer/")
            .and_then(|rest| rest.strip_suffix(".html"))
            .filter(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()))
    {
        ids.insert(id.to_string());
    }
    if let Some(platform) = platform
        && ids.len() == 1
    {
        let id = ids.iter().next()?;
        if crate::schema::is_album_id(id) {
            return Some(format!("{platform}:{id}"));
        }
    }
    if SHORTLINK_HOSTS.contains(&host.as_str()) && pystr::char_len(&path) > 1 {
        return Some(format!("url:{host}{path}"));
    }
    None
}

/// The marketplace identities a record's links name.
pub fn item_keys(links: &[Value]) -> BTreeSet<String> {
    links.iter().filter_map(item_key).collect()
}

pub fn duplicate_key(album: &Map) -> Option<String> {
    let links = album
        .get("links")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let keys = item_keys(&links);
    if keys.len() == 1 {
        return keys.into_iter().next().map(|k| format!("i:{k}"));
    }
    let title = match album.get("title") {
        Some(Value::Str(t)) => t.clone(),
        _ => String::new(),
    };
    let title: String = title
        .nfkc()
        .collect::<String>()
        .to_lowercase()
        .chars()
        .filter(|&c| pystr::is_letter_or_number(c))
        .collect();
    let weight: usize = title
        .chars()
        .map(|c| if c as u32 > 0x2e7f { 2 } else { 1 })
        .sum();
    if weight < 8 {
        return None;
    }
    let seller = album.get("seller").map(Value::py_str).unwrap_or_default();
    Some(format!("t:{seller}:{title}"))
}

/// Exact integer order of a digit string.
fn id_order(id: &str) -> (usize, String) {
    let trimmed = id.trim_start_matches('0');
    (trimmed.len(), trimmed.to_string())
}

fn rank(album: &Map) -> (Date, (usize, String)) {
    let id = album.get("id").and_then(Value::as_str).unwrap_or("0");
    let known = match album.get("date_published") {
        Some(Value::Str(s)) => s.as_str(),
        _ => "",
    };
    let prefix = pystr::prefix_chars(known, 10);
    let shaped = prefix.len() == 10
        && prefix.bytes().enumerate().all(|(i, b)| {
            if i == 4 || i == 7 {
                b == b'-'
            } else {
                b.is_ascii_digit()
            }
        });
    if shaped && let Some(date) = isodate::parse_date(prefix) {
        return (date, id_order(id));
    }
    let numeric: u128 = id.parse().unwrap_or(u128::MAX);
    (estimate_published(numeric), id_order(id))
}

/// Canonical records and older-ID aliases, without changing captures. A
/// title used over twice by one seller is a category label, as in dedup.js.
pub fn newest_listings(albums: &[Map]) -> (Vec<Map>, Vec<(String, String)>) {
    let albums: Vec<&Map> = albums.iter().filter(|a| canonical_album(a)).collect();
    let keys: Vec<Option<String>> = albums.iter().map(|a| duplicate_key(a)).collect();
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for key in keys.iter().flatten() {
        if key.starts_with("t:") {
            *counts.entry(key).or_default() += 1;
        }
    }
    let mut winners: HashMap<&str, usize> = HashMap::new();
    for (index, key) in keys.iter().enumerate() {
        let Some(key) = key else { continue };
        if counts.get(key.as_str()).copied().unwrap_or(0) > 2 {
            continue;
        }
        match winners.get(key.as_str()) {
            Some(&current) if rank(albums[index]) <= rank(albums[current]) => {}
            _ => {
                winners.insert(key, index);
            }
        }
    }
    let mut selected = Vec::new();
    let mut aliases = Vec::new();
    for (index, key) in keys.iter().enumerate() {
        let winner = key.as_deref().and_then(|k| winners.get(k)).copied();
        match winner {
            None => selected.push(albums[index].clone()),
            Some(w) if w == index => selected.push(albums[index].clone()),
            Some(w) => {
                let id = albums[index]
                    .get("id")
                    .map(Value::py_str)
                    .unwrap_or_default();
                let winner_id = albums[w].get("id").map(Value::py_str).unwrap_or_default();
                if id != winner_id {
                    // A repeated key keeps its first position, as in a dict.
                    match aliases
                        .iter_mut()
                        .find(|(old, _): &&mut (String, String)| *old == id)
                    {
                        Some(entry) => entry.1 = winner_id,
                        None => aliases.push((id, winner_id)),
                    }
                }
            }
        }
    }
    (selected, aliases)
}
