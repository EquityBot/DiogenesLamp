//! Small validation boundary for local catalogue and recovery files.

use std::path::{Path, PathBuf};

use idna::uts46::{AsciiDenyList, DnsLength, Hyphens, Uts46};

use crate::errors::{Error, Result, value};
use crate::ipaddr;
use crate::pyerr;
use crate::pyjson::{Map, Value};
use crate::pyurl;

fn digits_1_24(s: &str) -> bool {
    (1..=24).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit())
}

fn seller_id(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

pub fn is_album_id(s: &str) -> bool {
    digits_1_24(s)
}

pub fn is_seller_id(s: &str) -> bool {
    seller_id(s)
}

/// `thumbs/[a-z0-9_-]+/[0-9]+\.(?:jpg|png|webp)`.
pub fn is_cover_path(s: &str) -> bool {
    let Some(rest) = s.strip_prefix("thumbs/") else {
        return false;
    };
    let Some((seller, file)) = rest.split_once('/') else {
        return false;
    };
    let Some((stem, ext)) = file.rsplit_once('.') else {
        return false;
    };
    seller_id(seller)
        && !stem.is_empty()
        && stem.bytes().all(|b| b.is_ascii_digit())
        && matches!(ext, "jpg" | "png" | "webp")
}

/// `(?:index\.json|(?:catalog|runs)/[a-z0-9_-]+\.json)`.
fn is_metadata_path(s: &str) -> bool {
    if s == "index.json" {
        return true;
    }
    let Some((dir, file)) = s.split_once('/') else {
        return false;
    };
    let Some(stem) = file.strip_suffix(".json") else {
        return false;
    };
    matches!(dir, "catalog" | "runs") && seller_id(stem)
}

/// URL Standard numeric-host admission, including short IPv4 forms.
fn numeric_host(host: &str) -> Result<String> {
    let trimmed = host.strip_suffix('.').unwrap_or(host);
    let parts: Vec<&str> = trimmed.split('.').collect();
    let last = parts[parts.len() - 1];
    let numeric_last = (!last.is_empty() && last.bytes().all(|b| b.is_ascii_digit()))
        || last.strip_prefix("0x").is_some_and(|h| {
            h.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        });
    if !numeric_last {
        return Ok(host.to_string());
    }
    let invalid = || value("invalid IPv4 hostname");
    if parts.len() > 4 {
        return invalid();
    }
    let mut numbers: Vec<u128> = Vec::new();
    for part in &parts {
        if part.is_empty() {
            return invalid();
        }
        let (radix, digits) = if let Some(hex) = part.strip_prefix("0x") {
            (16, hex)
        } else if part.len() > 1 && part.starts_with('0') {
            (8, &part[1..])
        } else {
            (10, *part)
        };
        let ok = match radix {
            16 => digits
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            8 => digits.bytes().all(|b| (b'0'..=b'7').contains(&b)),
            _ => !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()),
        };
        if !ok {
            return invalid();
        }
        let n = if digits.is_empty() {
            0
        } else {
            // Values beyond u128 are far past every bound checked below.
            u128::from_str_radix(digits, radix).unwrap_or(u128::MAX)
        };
        numbers.push(n);
    }
    let count = numbers.len() as u32;
    let last = numbers[numbers.len() - 1];
    if numbers[..numbers.len() - 1].iter().any(|&n| n > 255) || last >= 256u128.pow(5 - count) {
        return invalid();
    }
    let mut address = last;
    for (i, n) in numbers[..numbers.len() - 1].iter().enumerate() {
        address += n * 256u128.pow(3 - i as u32);
    }
    Ok(ipaddr::v4_str(address as u32))
}

/// Admit canonical links without fetching or altering historical captures.
pub fn public_https_host(raw: &str) -> Result<String> {
    let refused = || value("canonical links require public HTTPS without credentials");
    let parsed = pyurl::urlsplit(&raw.replace('\\', "/")).map_err(Error::Value)?;
    let Some(hostname) = parsed.hostname() else {
        return refused();
    };
    if parsed.scheme != "https" || parsed.username().is_some() || parsed.password().is_some() {
        return refused();
    }
    parsed.port().map_err(Error::Value)?;
    let host = pyurl::unquote_with(&hostname, true)
        .map_err(Error::Value)?
        .to_lowercase();
    if parsed.netloc_str().starts_with('[') {
        if host.contains('%') {
            return value("IPv6 zone identifiers are not canonical URL hosts");
        }
        let v6 = ipaddr::parse_v6_value(&host).map_err(Error::Value)?;
        return Ok(ipaddr::v6_str(v6));
    }
    if host
        .chars()
        .any(|c| (c as u32) <= 32 || c as u32 == 127 || "#/:<>?@[\\]^|%".contains(c))
    {
        return value("invalid canonical hostname");
    }
    let mut host = host;
    if !host.is_ascii() || host.split('.').any(|label| label.starts_with("xn--")) {
        // shortcut: the idna crate's UTS 46 processing stands in for Python's
        // idna package (IDNA 2008 with UTS 46 mapping); their Unicode tables
        // and contextual rules can differ at the edges.
        // Revisit if a host is found to map differently from Python.
        host = Uts46::new()
            .to_ascii(
                host.as_bytes(),
                AsciiDenyList::EMPTY,
                Hyphens::Check,
                DnsLength::VerifyAllowRootDot,
            )
            .map_err(|_| Error::Value("invalid canonical hostname".into()))?
            .into_owned();
    }
    if host.is_empty() || host.starts_with(['*', '.']) {
        return value("invalid canonical hostname");
    }
    numeric_host(&host)
}

fn get<'a>(map: &'a Map, key: &str) -> Option<&'a Value> {
    map.get(key)
}

fn is_str(v: Option<&Value>) -> bool {
    matches!(v, Some(Value::Str(_)))
}

/// Keep legacy reading permissive; only admitted records compete for selection.
pub fn canonical_album(album: &Map) -> bool {
    let check = || -> Result<()> {
        let seller = album
            .get("seller")
            .ok_or_else(|| Error::Value("KeyError".into()))?;
        let seller_text = seller
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| seller.py_str());
        let mut shard = Map::new();
        shard.insert("seller".into(), seller.clone());
        shard.insert(
            "albums".into(),
            Value::Array(vec![Value::Object(album.clone())]),
        );
        validate_metadata(
            &Value::Object(shard),
            &format!("catalog/{seller_text}.json"),
        )?;
        let mut urls = vec![album.get("album_url").cloned().unwrap_or(Value::Null)];
        if let Some(Value::Array(links)) = album.get("links") {
            for link in links {
                urls.push(
                    link.as_object()
                        .and_then(|l| l.get("url"))
                        .cloned()
                        .unwrap_or(Value::Null),
                );
            }
        }
        for url in urls {
            let Value::Str(url) = url else {
                return value("TypeError");
            };
            public_https_host(&url)?;
        }
        Ok(())
    };
    check().is_ok()
}

/// Resolve and contain a catalogue path.
pub fn data_path(root: &Path, relative: &str, cover: bool) -> Result<PathBuf> {
    let safe = if cover {
        is_cover_path(relative)
    } else {
        is_metadata_path(relative)
    };
    if !safe {
        return value("unsafe catalogue path");
    }
    let root = pyerr::resolve(root);
    let path = pyerr::normpath(&pyerr::join_rel(&root, relative));
    if !pyerr::is_relative_to(&path, &root) {
        return value("catalogue path escapes data directory");
    }
    let mut existing = path.clone();
    while existing != root && std::fs::symlink_metadata(&existing).is_err() {
        match existing.parent() {
            Some(parent) => existing = parent.to_path_buf(),
            None => break,
        }
    }
    if !pyerr::is_relative_to(&pyerr::resolve(&existing), &root) {
        return value("catalogue path escapes data directory");
    }
    Ok(path)
}

fn url_ok(url: &str) -> Result<bool> {
    let parsed = pyurl::urlsplit(url).map_err(Error::Value)?;
    Ok(parsed.scheme == "https"
        && parsed.hostname().is_some()
        && parsed.username().is_none_or(|u| u.is_empty())
        && parsed.password().is_none_or(|p| p.is_empty()))
}

fn price_ok(price: &Value) -> bool {
    match price {
        Value::Int(i) => *i >= 0,
        Value::BigInt(s) => !s.starts_with('-') && s.parse::<f64>().is_ok_and(f64::is_finite),
        Value::Float(f) => f.is_finite() && *f >= 0.0,
        _ => false,
    }
}

fn schema_version_ok(map: &Map) -> bool {
    matches!(map.get("schema_version"), None | Some(Value::Int(1)))
}

pub fn validate_metadata(value_in: &Value, relative: &str) -> Result<()> {
    let Some(map) = value_in.as_object().filter(|m| schema_version_ok(m)) else {
        return value("unsupported metadata shape or schema version");
    };
    if relative == "index.json" {
        if let Some(v) = map.get("selection_version")
            && !matches!(v, Value::Int(1))
        {
            return value("unsupported selection version");
        }
        let Some(Value::Array(sellers)) = map.get("sellers") else {
            return value("index sellers must be an array");
        };
        let mut seen: Vec<&str> = Vec::new();
        let mut indexed: std::collections::HashSet<&str> = Default::default();
        let selected = matches!(map.get("selection_version"), Some(Value::Int(1)));
        for seller in sellers {
            let Some(s) = seller.as_object() else {
                return value("invalid seller ID");
            };
            let Some(Value::Str(id)) = get(s, "id") else {
                return value("invalid seller ID");
            };
            if !seller_id(id) {
                return value("invalid seller ID");
            }
            let shard_ok = matches!(get(s, "shard"), Some(Value::Str(p)) if *p == format!("catalog/{id}.json"));
            if seen.contains(&id.as_str()) || !shard_ok {
                return value("duplicate seller or mismatched shard path");
            }
            seen.push(id);
            if let Some(ids) = get(s, "album_ids") {
                let Some(ids) = ids.as_array() else {
                    return value("invalid indexed album IDs");
                };
                let mut texts: Vec<&str> = Vec::new();
                for i in ids {
                    match i {
                        Value::Str(t) if digits_1_24(t) => texts.push(t),
                        _ => return value("invalid indexed album IDs"),
                    }
                }
                let unique: std::collections::HashSet<&str> = texts.iter().copied().collect();
                if unique.len() != texts.len() {
                    return value("invalid indexed album IDs");
                }
                if texts.iter().any(|t| indexed.contains(t)) {
                    return value("duplicate indexed album ID across sellers");
                }
                indexed.extend(texts);
            } else if selected {
                return value("selected index requires album IDs for every seller");
            }
        }
        let aliases_ok = match map.get("album_aliases") {
            None => true,
            Some(Value::Object(aliases)) => aliases.iter().all(|(old, new)| {
                digits_1_24(old)
                    && matches!(new, Value::Str(n) if indexed.contains(n.as_str()))
                    && !indexed.contains(old.as_str())
            }),
            Some(_) => false,
        };
        if !aliases_ok {
            return value("invalid album aliases");
        }
        return Ok(());
    }
    if relative.starts_with("runs/") {
        let ids_ok = match map.get("failed_ids") {
            None => true,
            Some(Value::Array(ids)) => ids
                .iter()
                .all(|i| matches!(i, Value::Str(t) if crate::pystr::all_digits(t))),
            Some(_) => false,
        };
        if !ids_ok {
            return value("invalid failed album IDs");
        }
        let missing_ok = match map.get("short_missing") {
            None => true,
            Some(Value::Array(ids)) => ids
                .iter()
                .all(|i| matches!(i, Value::Str(t) if crate::pystr::all_digits(t))),
            Some(_) => false,
        };
        if !missing_ok {
            return value("invalid short_missing album IDs");
        }
        for field in ["last_attempt", "last_success", "last_listing"] {
            if let Some(v) = map.get(field)
                && !matches!(v, Value::Str(_))
            {
                return value(format!("invalid {field} timestamp"));
            }
        }
        return Ok(());
    }
    let seller = Path::new(relative)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    if !matches!(map.get("seller"), Some(Value::Str(s)) if *s == seller) {
        return value("shard seller does not match its path");
    }
    let Some(Value::Array(albums)) = map.get("albums") else {
        return value("shard albums must be an array");
    };
    let mut seen: std::collections::HashSet<String> = Default::default();
    for album in albums {
        let Some(a) = album.as_object() else {
            return value("invalid album ID");
        };
        let id_text = match a.get("id") {
            None => String::new(),
            Some(v) => v.py_str(),
        };
        let id_like = match a.get("id") {
            Some(Value::Str(_) | Value::Int(_) | Value::BigInt(_)) => digits_1_24(&id_text),
            _ => false,
        };
        if !id_like {
            return value("invalid album ID");
        }
        let Some(Value::Str(id)) = a.get("id") else {
            return value("duplicate or non-string album ID");
        };
        if !seen.insert(id.clone()) {
            return value("duplicate or non-string album ID");
        }
        if !matches!(a.get("seller"), Some(Value::Str(s)) if *s == seller) {
            return value("album seller does not match its shard");
        }
        for field in ["title", "thumb", "album_url"] {
            if !is_str(a.get(field)) {
                return value(format!("album {field} must be text"));
            }
        }
        for field in [
            "category",
            "description",
            "first_seen",
            "updated_at",
            "thumb_version",
            "cover_hash",
        ] {
            if a.get(field).is_some_and(|v| !matches!(v, Value::Str(_))) {
                return value(format!("album {field} must be text"));
            }
        }
        if a.get("description_truncated")
            .is_some_and(|v| !matches!(v, Value::Bool(_)))
        {
            return value("album description_truncated must be a boolean");
        }
        if let Some(price) = a.get("price")
            && !price_ok(price)
        {
            return value("invalid price");
        }
        if let Some(Value::Str(thumb)) = a.get("thumb")
            && !thumb.is_empty()
            && !is_cover_path(thumb)
        {
            return value("unsafe cover path");
        }
        let Some(Value::Array(links)) = a.get("links") else {
            return value("purchase links must be an array");
        };
        let mut urls = vec![
            a.get("album_url")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        ];
        for link in links {
            let ok = link
                .as_object()
                .is_some_and(|l| is_str(l.get("url")) && is_str(l.get("platform")));
            if !ok {
                return value("invalid purchase link");
            }
            urls.push(
                link.as_object()
                    .and_then(|l| l.get("url"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            );
        }
        for url in &urls {
            if !url_ok(url)? {
                return value("album links must be public HTTPS URLs without credentials");
            }
        }
        if let Some(Value::Object(evidence)) = a.get("price_evidence")
            && evidence
                .get("candidates")
                .is_some_and(|c| !matches!(c, Value::Array(_)))
        {
            return value("price candidates must be an array");
        }
        if let Some(history) = a.get("history") {
            let ok = history.as_array().is_some_and(|items| {
                items
                    .iter()
                    .all(|h| h.as_object().is_some_and(|h| is_str(h.get("at"))))
            });
            if !ok {
                return value("invalid change history");
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_hosts_follow_the_url_standard() {
        assert_eq!(public_https_host("https://127.1/").unwrap(), "127.0.0.1");
        assert_eq!(public_https_host("https://0x7f.01/").unwrap(), "127.0.0.1");
        assert_eq!(
            public_https_host("https://2130706433/").unwrap(),
            "127.0.0.1"
        );
        assert_eq!(
            public_https_host("https://127.0.0.1./").unwrap(),
            "127.0.0.1"
        );
        assert!(public_https_host("https://256.1.1.1/").is_err());
        assert!(public_https_host("https://1.2.3.4.5/").is_err());
        assert!(public_https_host("https://example.09/").is_err());
        assert!(public_https_host("https://4294967296/").is_err());
        assert_eq!(public_https_host("https://0x/").unwrap(), "0.0.0.0");
        assert_eq!(public_https_host("https://[::1]/").unwrap(), "::1");
        assert_eq!(
            public_https_host("https://%77eidian.com/").unwrap(),
            "weidian.com"
        );
    }
}
