//! Parsers for Yupoo album list pages and album pages.

use std::sync::LazyLock;

use regex::Regex;

use crate::html::{self, Document, NodeId};
use crate::pyjson::{self, Map, Value};
use crate::pystr;

/// Python's `\s` as a class body.
const PY_SPACE: &str = r"\t\n\x0b\x0c\r\x1c-\x20\x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}";
const AMOUNT: &str = r"(?:[0-9]{1,3}(?:,[0-9]{3})+|[0-9]+)(?:\.[0-9]+)?";

struct PricePattern {
    re: Regex,
    /// `\b` before the match.
    word_start: bool,
    /// `\b` after the match.
    word_end: bool,
    /// `(?![0-9])` after the match.
    no_digit_after: bool,
}

static PRICE_PATTERNS: LazyLock<Vec<PricePattern>> = LazyLock::new(|| {
    let p = |re: String, word_start, word_end, no_digit_after| PricePattern {
        re: Regex::new(&re).expect("price pattern compiles"),
        word_start,
        word_end,
        no_digit_after,
    };
    vec![
        p(
            format!("(?:¥|￥)[{PY_SPACE}]*({AMOUNT})"),
            false,
            false,
            false,
        ),
        p(
            format!("(?:[rR][mM][bB]|[yY][uU][aA][nN])[{PY_SPACE}]*({AMOUNT})"),
            true,
            false,
            false,
        ),
        p(
            format!("({AMOUNT})[{PY_SPACE}]*(?:元|块)"),
            false,
            false,
            false,
        ),
        p(
            format!("({AMOUNT})[{PY_SPACE}]*(?:[rR][mM][bB]|[yY][uU][aA][nN])"),
            false,
            true,
            false,
        ),
        p("[pP]([0-9]{2,4})".into(), true, false, true),
        p("([0-9]{2,4})[pP]".into(), true, true, false),
    ]
});

/// `re.finditer` for one pattern, with Python's boundaries.
fn find_all<'t>(pattern: &PricePattern, text: &'t str) -> Vec<(&'t str, &'t str)> {
    let mut found = Vec::new();
    let mut pos = 0;
    while pos <= text.len() {
        let Some(caps) = pattern.re.captures_at(text, pos) else {
            break;
        };
        let whole = caps.get(0).expect("group 0");
        let (start, end) = (whole.start(), whole.end());
        let before = text[..start].chars().next_back();
        let after = text[end..].chars().next();
        let ok = (!pattern.word_start || !before.is_some_and(pystr::is_word))
            && (!pattern.word_end || !after.is_some_and(pystr::is_word))
            && (!pattern.no_digit_after || !after.is_some_and(|c| c.is_ascii_digit()));
        if ok {
            found.push((whole.as_str(), caps.get(1).map_or("", |m| m.as_str())));
            pos = end;
        } else {
            pos = start + text[start..].chars().next().map_or(1, char::len_utf8);
        }
    }
    found
}

/// Whole yuan, rounded half up, within the catalogue range 10..=99999.
fn cny_amount(raw: &str) -> Option<i64> {
    let raw = raw.replace(',', "");
    let (int_part, frac) = raw.split_once('.').unwrap_or((&raw, ""));
    let int_part = int_part.trim_start_matches('0');
    if int_part.len() > 6 {
        return None;
    }
    let mut amount: i64 = if int_part.is_empty() {
        0
    } else {
        int_part.parse().ok()?
    };
    if frac.bytes().next().is_some_and(|d| d >= b'5') {
        amount += 1;
    }
    (10..=99999).contains(&amount).then_some(amount)
}

/// The matched token and rule, with competing amounts made explicit.
pub fn price_evidence(text: &str) -> Map {
    let mut evidence = Map::new();
    let mut candidates: Vec<i64> = Vec::new();
    for (index, pattern) in PRICE_PATTERNS.iter().enumerate() {
        for (token, group) in find_all(pattern, text) {
            let Some(amount) = cny_amount(group) else {
                continue;
            };
            if evidence.is_empty() {
                evidence.insert("amount".into(), amount.into());
                evidence.insert("token".into(), pystr::prefix_chars(token, 100).into());
                evidence.insert(
                    "rule".into(),
                    if index < 4 { "currency" } else { "shorthand" }.into(),
                );
            }
            if !candidates.contains(&amount) && candidates.len() < 10 {
                candidates.push(amount);
            }
        }
    }
    if !evidence.is_empty() {
        evidence.insert(
            "candidates".into(),
            Value::Array(candidates.into_iter().map(Value::from).collect()),
        );
    }
    evidence
}

/// Price in CNY from title or description text, or 0.
pub fn parse_price(text: &str) -> i64 {
    match price_evidence(text).get("amount") {
        Some(Value::Int(n)) => *n as i64,
        _ => 0,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AlbumStub {
    pub id: String,
    pub title: String,
    pub thumb_url: String,
    pub photo_count: i128,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AlbumDetail {
    pub title: String,
    pub description: String,
    pub category: String,
    pub photo_count: i128,
    pub date_published: String,
    pub date_modified: String,
    pub price: i64,
}

fn absolute(url: &str) -> String {
    match url.strip_prefix("//") {
        Some(rest) => format!("https://{rest}"),
        None => url.to_string(),
    }
}

/// `re.search(r"\d+", text)` with Unicode digits, as an int.
fn first_int(text: &str) -> i128 {
    let mut digits = String::new();
    for c in text.chars() {
        if pystr::decimal_value(c).is_some() {
            digits.push(c);
        } else if !digits.is_empty() {
            break;
        }
    }
    pystr::parse_int(&digits).unwrap_or(0)
}

/// `/albums/(\d+)` searched anywhere, with Unicode digits.
pub fn album_href_id(href: &str) -> Option<String> {
    let mut search = href;
    while let Some(pos) = search.find("/albums/") {
        let rest = &search[pos + 8..];
        let id: String = rest
            .chars()
            .take_while(|&c| pystr::decimal_value(c).is_some())
            .collect();
        if !id.is_empty() {
            return Some(id);
        }
        search = &search[pos + 1..];
    }
    None
}

fn is_placeholder(url: &str) -> bool {
    url.rsplit('/').next() == Some("im_photo_album.png")
}

fn card_thumb(doc: &Document, card: NodeId) -> String {
    for selector in ["img.album__img", "img.album3__img", "img"] {
        for img in doc.select(card, selector) {
            for attr in ["data-src", "data-origin-src", "src"] {
                let value = pystr::strip(doc.attr(img, attr).unwrap_or(""));
                if value.is_empty() || value.starts_with("data:") || is_placeholder(value) {
                    continue;
                }
                return absolute(value);
            }
        }
    }
    String::new()
}

pub fn parse_album_list(html_text: &str) -> Vec<AlbumStub> {
    let doc = html::parse(html_text);
    let mut stubs = Vec::new();
    for card in doc.select(doc.root(), "a.album__main, a.album3__main") {
        let Some(id) = album_href_id(doc.attr(card, "href").unwrap_or("")) else {
            continue;
        };
        let title_attr = doc.attr(card, "title").unwrap_or("");
        let title = if title_attr.is_empty() {
            doc.select_one(card, ".album__title")
                .map(|el| doc.get_text(el, "", true))
                .unwrap_or_default()
        } else {
            title_attr.to_string()
        };
        let count = doc
            .select_one(card, ".album__count, .album__photonumber")
            .map(|el| doc.get_text(el, "", false))
            .unwrap_or_default();
        stubs.push(AlbumStub {
            id,
            title: pystr::strip(&title).to_string(),
            thumb_url: card_thumb(&doc, card),
            photo_count: first_int(&count),
        });
    }
    stubs
}

fn ld_json_objects(doc: &Document) -> Vec<Map> {
    let mut objects = Vec::new();
    for script in doc.select(doc.root(), "script[type=\"application/ld+json\"]") {
        let Ok(payload) = pyjson::loads(&doc.get_text(script, "", false)) else {
            continue;
        };
        let mut queue = std::collections::VecDeque::from([payload]);
        while let Some(node) = queue.pop_front() {
            match node {
                Value::Array(items) => queue.extend(items),
                Value::Object(map) => {
                    if let Some(graph @ (Value::Array(_) | Value::Object(_))) = map.get("@graph") {
                        queue.push_back(graph.clone());
                    }
                    objects.push(map);
                }
                _ => {}
            }
        }
    }
    objects
}

/// `^\d{4}-\d{2}-\d{2}` with Unicode digits.
fn date_shaped(value: &str) -> bool {
    let chars: Vec<char> = value.chars().take(10).collect();
    chars.len() == 10
        && chars.iter().enumerate().all(|(i, &c)| {
            if i == 4 || i == 7 {
                c == '-'
            } else {
                pystr::decimal_value(c).is_some()
            }
        })
}

fn dates_from_ld_json(doc: &Document) -> (String, String) {
    let (mut published, mut modified) = (String::new(), String::new());
    for node in ld_json_objects(doc) {
        for (key, slot) in [
            ("datePublished", &mut published),
            ("dateModified", &mut modified),
        ] {
            if slot.is_empty() {
                let value = match node.get(key) {
                    Some(v) if v.truthy() => v.py_str(),
                    _ => String::new(),
                };
                if date_shaped(&value) {
                    *slot = value;
                }
            }
        }
        if !published.is_empty() && !modified.is_empty() {
            break;
        }
    }
    (published, modified)
}

pub fn parse_album_page(html_text: &str) -> AlbumDetail {
    let doc = html::parse(html_text);
    let root = doc.root();
    let title = match doc.select_one(root, ".showalbumheader__gallerytitle") {
        Some(el) => {
            let name = doc.attr(el, "data-name").unwrap_or("");
            if name.is_empty() {
                doc.get_text(el, "", true)
            } else {
                name.to_string()
            }
        }
        None => String::new(),
    };
    let description = doc
        .select_one(root, ".showalbumheader__gallerysubtitle")
        .map(|el| doc.get_text(el, "\n", true))
        .unwrap_or_default();
    let category = doc
        .select_one(root, ".showalbumheader__categorieslink")
        .map(|el| doc.get_text(el, "", true))
        .unwrap_or_default();
    let count = doc
        .select_one(root, ".showalbumheader__gallerycount")
        .map(|el| doc.get_text(el, "", false))
        .unwrap_or_default();
    let (published, modified) = dates_from_ld_json(&doc);
    let price = parse_price(&format!("{title} {description}"));
    AlbumDetail {
        title: pystr::strip(&title).to_string(),
        description,
        category,
        photo_count: first_int(&count),
        date_published: published,
        date_modified: modified,
        price,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prices_match_python() {
        for (text, amount) in [
            ("Nike Tech Fleece ¥180 Black", 180),
            ("Stussy Tee ￥150 White", 150),
            ("Bape Hoodie 220元 M-XXL", 220),
            ("Balenciaga Shoes P350 40-45", 350),
            ("Prada Jacket 420P New", 420),
            ("Arcteryx 380 RMB", 380),
            ("Plain Title No Price", 0),
            ("Size 180cm height", 0),
            ("Micro price 5", 0),
            ("Nike Tech Fleece ¥180.50 Black", 181),
            ("180.50 yuan", 181),
            ("Jacket ¥1,280", 1280),
            ("¥180黑色", 180),
            ("黑色180元新款", 180),
            ("¥5 ¥180", 180),
            ("¥8.8 ¥180", 180),
            ("¥100000 then ¥180", 180),
        ] {
            assert_eq!(parse_price(text), amount, "{text}");
        }
    }
}
