//! Ingest raw link dumps into sources.toml.

use std::path::Path;
use std::sync::LazyLock;

use indexmap::IndexMap;
use regex::Regex;

use crate::errors::{Error, Result, value};
use crate::pyerr::{self, Call};
use crate::pyjson::{self, Value};
use crate::pystr;
use crate::pyurl;

const YUPOO_SUFFIX: &str = ".x.yupoo.com";

/// A seller or shop. `skip = true` in sources.toml leaves a seller out of
/// scrapes. Name, URL and platform keep whatever TOML type the file gave
/// them, as Python does.
#[derive(Clone, Debug)]
pub struct Source {
    pub kind: String,
    pub id: String,
    pub platform: Value,
    pub url: Value,
    pub name: Value,
    pub skip: bool,
}

impl PartialEq for Source {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind
            && self.id == other.id
            && pyjson::py_eq(&self.platform, &other.platform)
            && pyjson::py_eq(&self.url, &other.url)
            && pyjson::py_eq(&self.name, &other.name)
            && self.skip == other.skip
    }
}

impl Source {
    pub fn new(kind: &str, id: &str, platform: &str, url: &str, name: &str) -> Source {
        Source {
            kind: kind.into(),
            id: id.into(),
            platform: platform.into(),
            url: url.into(),
            name: name.into(),
            skip: false,
        }
    }

    pub fn url_str(&self) -> String {
        self.url.py_str()
    }
}

pub type Sources = IndexMap<String, Source>;

fn slug(text: &str) -> String {
    let mut out = String::new();
    let mut in_run = false;
    for c in text.to_lowercase().chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' {
            out.push(c);
            in_run = false;
        } else if !in_run {
            out.push('-');
            in_run = true;
        }
    }
    let trimmed = out.trim_matches('-');
    let mut collapsed = String::new();
    for c in trimmed.chars() {
        if c == '-' && collapsed.ends_with('-') {
            continue;
        }
        collapsed.push(c);
    }
    collapsed
}

fn strip_quotes(raw: &str) -> &str {
    if raw.len() >= 2
        && ((raw.starts_with('"') && raw.ends_with('"'))
            || (raw.starts_with('\'') && raw.ends_with('\'')))
    {
        pystr::strip(&raw[1..raw.len() - 1])
    } else if raw == "\"" || raw == "'" {
        ""
    } else {
        raw
    }
}

fn is_invisible(c: char) -> bool {
    matches!(
        c,
        '\x00'..='\x1f' | '\x7f' | '\u{feff}' | '\u{200b}' | '\u{200c}' | '\u{200d}' | '\u{2060}'
    )
}

const USES_PARAMS: [&str; 16] = [
    "", "ftp", "hdl", "prospero", "http", "imap", "https", "shttp", "rtsp", "rtsps", "rtspu",
    "sip", "sips", "mms", "sftp", "tel",
];

/// `urlparse(url).path`: urlsplit's path less any `;params`.
fn urlparse_path(split: &pyurl::Split) -> String {
    let path = &split.path;
    if USES_PARAMS.contains(&split.scheme.as_str()) && path.contains(';') {
        let cut = if let Some(slash) = path.rfind('/') {
            path[slash..].find(';').map(|i| i + slash)
        } else {
            path.find(';')
        };
        if let Some(cut) = cut {
            return path[..cut].to_string();
        }
    }
    path.clone()
}

pub fn classify_url(raw: &str) -> Option<Source> {
    let raw = strip_quotes(pystr::strip(raw));
    if raw.is_empty() || raw.to_uppercase() == "N/A" {
        return None;
    }
    let mut raw: String = raw.chars().filter(|&c| !is_invisible(c)).collect();
    if raw.is_empty() {
        return None;
    }
    if !raw.contains("://") {
        raw = format!("https://{raw}");
    }
    let parsed = pyurl::urlsplit(&raw).ok()?;
    if !matches!(parsed.scheme.to_lowercase().as_str(), "http" | "https") {
        return None;
    }
    if parsed.username().is_some() || parsed.password().is_some() {
        return None;
    }
    let host = parsed.hostname().unwrap_or_default().to_lowercase();
    if host.is_empty() || !host.contains('.') {
        return None;
    }
    let path = urlparse_path(&parsed);
    if let Some(seller) = host.strip_suffix(YUPOO_SUFFIX) {
        if seller.is_empty() {
            return None;
        }
        let seller_id = slug(&seller.replace('.', "-"));
        return Some(Source::new(
            "yupoo",
            &seller_id,
            "yupoo",
            &format!("https://{host}"),
            &seller_id,
        ));
    }
    if host == "taobao.com" || host.ends_with(".taobao.com") {
        let stem = host.strip_suffix(".taobao.com").unwrap_or(&host);
        let stem = if stem.is_empty() { "taobao" } else { stem };
        return Some(Source::new(
            "shop",
            &format!("taobao-{}", slug(stem)),
            "taobao",
            &format!("https://{host}/"),
            stem,
        ));
    }
    if host == "weidian.com" {
        let userid = pyurl::parse_qsl(parsed.query_str())
            .into_iter()
            .find(|(k, _)| k == "userid")
            .map(|(_, v)| v);
        if let Some(userid) =
            userid.filter(|u| !u.is_empty() && u.bytes().all(|b| b.is_ascii_digit()))
        {
            return Some(Source::new(
                "shop",
                &format!("weidian-{userid}"),
                "weidian",
                &format!("https://weidian.com/?userid={userid}"),
                &format!("weidian {userid}"),
            ));
        }
        return Some(Source::new(
            "shop",
            "weidian-com",
            "web",
            &raw,
            "weidian.com",
        ));
    }
    if host == "k.youshop10.com" {
        let code = path.trim_matches('/');
        if code.is_empty() {
            return None;
        }
        return Some(Source::new(
            "shop",
            &format!("youshop-{}", slug(code)),
            "weidian-shortlink",
            &raw,
            code,
        ));
    }
    if host == "vlink.cc" || host == "linktr.ee" {
        let stem = host.split('.').next().unwrap_or("");
        let trimmed = path.trim_matches('/');
        let page = slug(trimmed);
        let page = if page.is_empty() {
            "home".to_string()
        } else {
            page
        };
        return Some(Source::new(
            "shop",
            &format!("{stem}-{page}"),
            "aggregator",
            &raw,
            &format!("{stem}/{trimmed}"),
        ));
    }
    let name = host.strip_prefix("www.").unwrap_or(&host);
    Some(Source::new("shop", &slug(name), "web", &raw, name))
}

#[derive(Debug, Default)]
pub struct ImportReport {
    pub rows: usize,
    pub invalid: usize,
    pub yupoo_new: usize,
    pub yupoo_known: usize,
    pub shops_new: usize,
    pub shops_known: usize,
    pub shops_by_platform: Vec<(String, usize)>,
    /// (key, kept URL, dropped URL) for ids two different hosts slugged to.
    pub collisions: Vec<(String, String, String)>,
}

static INDEX_PREFIX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"^(?:(?:"[0-9]+"|[0-9]+)[\t,][\t\n\x0b\x0c\r\x1c-\x20\x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}]*)+(?P<url>[^\n]+)$"#,
    )
    .expect("dump pattern compiles")
});

pub fn parse_dump(text: &str) -> Vec<String> {
    let mut rows = Vec::new();
    for line in pystr::splitlines(text) {
        let line = pystr::strip(line).trim_start_matches('\u{feff}');
        if line.is_empty() {
            continue;
        }
        let extracted = INDEX_PREFIX
            .captures(line)
            .and_then(|c| c.name("url").map(|m| m.as_str()))
            .unwrap_or(line);
        rows.push(strip_quotes(pystr::strip(extracted)).to_string());
    }
    rows
}

pub fn merge_sources(existing: &Sources, raws: &[String]) -> (Sources, ImportReport) {
    let mut merged = existing.clone();
    let mut report = ImportReport {
        rows: raws.len(),
        ..Default::default()
    };
    let mut seen: std::collections::HashSet<String> = Default::default();
    for raw in raws {
        let Some(src) = classify_url(raw) else {
            report.invalid += 1;
            continue;
        };
        let key = format!("{}:{}", src.kind, src.id);
        if merged.contains_key(&key) || seen.contains(&key) {
            if let Some(held) = merged.get(&key)
                && !pyjson::py_eq(&held.url, &src.url)
            {
                report
                    .collisions
                    .push((key.clone(), held.url.py_str(), src.url.py_str()));
            }
            if !seen.contains(&key) {
                if src.kind == "yupoo" {
                    report.yupoo_known += 1;
                } else {
                    report.shops_known += 1;
                }
            }
            seen.insert(key);
            continue;
        }
        seen.insert(key.clone());
        if src.kind == "yupoo" {
            report.yupoo_new += 1;
        } else {
            report.shops_new += 1;
            let platform = src.platform.py_str();
            match report
                .shops_by_platform
                .iter_mut()
                .find(|(p, _)| *p == platform)
            {
                Some(entry) => entry.1 += 1,
                None => report.shops_by_platform.push((platform, 1)),
            }
        }
        merged.insert(key, src);
    }
    (merged, report)
}

/// `_toml_str`: a JSON string literal, which TOML reads back, with DEL escaped.
fn toml_value(value: &Value) -> String {
    pyjson::dumps(value, pyjson::Style::Default).replace('\u{7f}', "\\u007f")
}

fn safe_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// `Path.with_suffix(".toml.tmp")`.
fn tmp_path(path: &Path) -> std::path::PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stem = match name.rfind('.') {
        Some(i) if i > 0 && i < name.len() - 1 => &name[..i],
        _ => name.as_str(),
    };
    path.with_file_name(format!("{stem}.toml.tmp"))
}

pub fn write_sources(path: &Path, sources: &Sources) -> Result<()> {
    let mut lines: Vec<String> = vec![
        "# Managed by 'lamp import'. Manual edits to name/url are preserved.".into(),
        "# Add 'skip = true' to an entry to leave it out of scrapes.".into(),
        String::new(),
    ];
    let mut keys: Vec<&String> = sources.keys().collect();
    keys.sort();
    for key in keys {
        let src = &sources[key];
        if !safe_id(&src.id) {
            return value(format!("unsafe TOML id: {}", pystr::repr(&src.id)));
        }
        let section = if src.kind == "yupoo" {
            "yupoo"
        } else {
            "shops"
        };
        lines.push(format!("[{section}.{}]", src.id));
        if src.kind != "yupoo" {
            lines.push(format!("platform = {}", toml_value(&src.platform)));
        }
        lines.push(format!("url = {}", toml_value(&src.url)));
        lines.push(format!("name = {}", toml_value(&src.name)));
        if src.skip {
            lines.push("skip = true".into());
        }
        lines.push(String::new());
    }
    let tmp = tmp_path(path);
    let text = pyerr::native_newlines(&lines.join("\n"));
    std::fs::write(&tmp, text).map_err(|e| Error::Os(pyerr::os_error(&e, Call::Open, &tmp)))?;
    std::fs::rename(&tmp, path).map_err(|e| Error::Os(pyerr::os_error2(&e, Call::Os, &tmp, path)))
}

fn toml_to_value(value: &toml::Value) -> Value {
    match value {
        toml::Value::String(s) => Value::Str(s.clone()),
        toml::Value::Integer(i) => Value::Int(i128::from(*i)),
        toml::Value::Float(f) => Value::Float(*f),
        toml::Value::Boolean(b) => Value::Bool(*b),
        // shortcut: tomllib gives a datetime object, which Python can only
        // print as text; keep the TOML text. Keep a datetime type if a consumer needs one.
        toml::Value::Datetime(d) => Value::Str(d.to_string()),
        toml::Value::Array(items) => Value::Array(items.iter().map(toml_to_value).collect()),
        toml::Value::Table(table) => Value::Object(
            table
                .iter()
                .map(|(k, v)| (k.clone(), toml_to_value(v)))
                .collect(),
        ),
    }
}

pub fn load_sources(path: &Path) -> Result<Sources> {
    if !path.exists() {
        return Ok(Sources::new());
    }
    let text = pyerr::read_text(path).map_err(|f| Error::Value(f.message))?;
    let data: toml::Table = text.parse().map_err(|e: toml::de::Error| {
        Error::Value(format!(
            "invalid sources file {}: {}",
            pyerr::path_str(path),
            e.message()
        ))
    })?;
    let mut sources = Sources::new();
    let section = |name: &str| -> Result<Vec<(String, toml::Table)>> {
        match data.get(name) {
            None => Ok(Vec::new()),
            Some(toml::Value::Table(table)) => table
                .iter()
                .map(|(k, v)| match v {
                    toml::Value::Table(entry) => Ok((k.clone(), entry.clone())),
                    _ => value(format!("[{name}.{k}] must be a table")),
                })
                .collect(),
            Some(_) => value(format!("[{name}] must be a table")),
        }
    };
    let truthy = |entry: &toml::Table| {
        entry
            .get("skip")
            .map(toml_to_value)
            .is_some_and(|v| v.truthy())
    };
    for (seller_id, entry) in section("yupoo")? {
        if !safe_id(&seller_id) {
            return value(format!("unsafe TOML id: {}", pystr::repr(&seller_id)));
        }
        let Some(toml::Value::String(url)) = entry.get("url") else {
            return value(format!("[yupoo.{seller_id}] needs a url string"));
        };
        let name = entry
            .get("name")
            .map(toml_to_value)
            .unwrap_or_else(|| seller_id.as_str().into());
        sources.insert(
            format!("yupoo:{seller_id}"),
            Source {
                kind: "yupoo".into(),
                id: seller_id.clone(),
                platform: "yupoo".into(),
                url: url.trim_end_matches('/').into(),
                name,
                skip: truthy(&entry),
            },
        );
    }
    for (shop_id, entry) in section("shops")? {
        if !safe_id(&shop_id) {
            return value(format!("unsafe TOML id: {}", pystr::repr(&shop_id)));
        }
        let Some(url) = entry.get("url") else {
            return value(format!("[shops.{shop_id}] needs a url"));
        };
        sources.insert(
            format!("shop:{shop_id}"),
            Source {
                kind: "shop".into(),
                id: shop_id.clone(),
                platform: entry
                    .get("platform")
                    .map(toml_to_value)
                    .unwrap_or_else(|| "web".into()),
                url: toml_to_value(url),
                name: entry
                    .get("name")
                    .map(toml_to_value)
                    .unwrap_or_else(|| shop_id.as_str().into()),
                skip: truthy(&entry),
            },
        );
    }
    Ok(sources)
}
