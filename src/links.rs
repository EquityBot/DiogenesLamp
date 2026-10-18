//! Extract purchase links from album text.
//!
//! Marketplace links (taobao, tmall, weidian, 1688) are normalised to the
//! item they name. Shopping-agent links wrap the same marketplace item, so
//! they are unwrapped to it: the record carries the marketplace link, which
//! outlives the agent, and the viewer rebuilds a link for whichever agent the
//! operator prefers.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::LazyLock;

use fancy_regex::Regex as FancyRegex;
use regex::Regex;

use crate::pystr;
use crate::pyurl;

pub const MARKETPLACE_HOSTS: [&str; 4] = ["taobao.com", "tmall.com", "weidian.com", "1688.com"];
pub const SHORTLINK_HOSTS: [&str; 2] = ["m.tb.cn", "k.youshop10.com"];
/// Agents whose product links carry a marketplace item. viewer/agents.js
/// lists the same hosts; a parity test keeps the two equal.
pub const AGENT_HOSTS: [&str; 25] = [
    "kakobuy.com",
    "hoobuy.com",
    "oopbuy.com",
    "mulebuy.com",
    "joyagoo.com",
    "cssbuy.com",
    "sugargoo.com",
    "superbuy.com",
    "loongbuy.com",
    "pikobuy.com",
    "cnfans.com",
    "allchinabuy.com",
    "pandabuy.com",
    "itaobuy.com",
    "bbdbuy.com",
    "ootdbuy.com",
    "lovegobuy.com",
    "cnshopper.com",
    "alleurobuy.com",
    "rizzitgo.com",
    "vigorbuy.com",
    "usfans.com",
    "litbuy.com",
    "hipobuy.com",
    "gtbuy.com",
];

/// The item page each marketplace serves for a bare item id.
pub const ITEM_URLS: [(&str, &str); 4] = [
    ("taobao", "https://item.taobao.com/item.htm?id={id}"),
    ("tmall", "https://detail.tmall.com/item.htm?id={id}"),
    ("weidian", "https://weidian.com/item.html?itemID={id}"),
    ("1688", "https://detail.1688.com/offer/{id}.html"),
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub platform: String,
    pub url: String,
}

impl Link {
    fn new(platform: &str, url: String) -> Link {
        Link {
            platform: platform.into(),
            url,
        }
    }
}

/// Python's `\s` as a class body.
const PY_SPACE: &str = r"\t\n\x0b\x0c\r\x1c-\x20\x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}";
/// `[A-Za-z]` under Python's IGNORECASE.
const PY_LETTERS: &str = r"A-Za-z\x{130}\x{131}\x{17f}\x{212a}";

fn ignorecase_literal(host: &str) -> String {
    host.chars()
        .map(|c| match c.to_ascii_lowercase() {
            'i' => r"[iI\x{130}\x{131}]".to_string(),
            's' => r"[sS\x{17f}]".to_string(),
            'k' => r"[kK\x{212a}]".to_string(),
            l if l.is_ascii_alphabetic() => format!("[{l}{}]", l.to_ascii_uppercase()),
            '.' => r"\.".to_string(),
            '-' => r"\-".to_string(),
            d => d.to_string(),
        })
        .collect()
}

fn url_pattern() -> String {
    let alternation: Vec<String> = MARKETPLACE_HOSTS
        .iter()
        .chain(SHORTLINK_HOSTS.iter())
        .chain(AGENT_HOSTS.iter())
        .map(|h| ignorecase_literal(h))
        .collect();
    let label = format!("[{PY_LETTERS}0-9-]");
    format!(
        r#"(?<![{PY_LETTERS}0-9@-])(?:{h}{t}{t}{p}{s}?://)?(?P<host>(?:{label}+\.)*(?:{alt}))(?!\.?{label}|@)(?P<rest>/[^{PY_SPACE}'"<>]*|\?[^{PY_SPACE}'"<>]*)?"#,
        h = ignorecase_literal("h"),
        t = ignorecase_literal("t"),
        p = ignorecase_literal("p"),
        s = ignorecase_literal("s"),
        alt = alternation.join("|"),
    )
}

static URL_RE: LazyLock<FancyRegex> =
    LazyLock::new(|| FancyRegex::new(&url_pattern()).expect("URL pattern compiles"));
static URL_FULL_RE: LazyLock<FancyRegex> = LazyLock::new(|| {
    FancyRegex::new(&format!("^(?:{})$", url_pattern())).expect("URL pattern compiles")
});
static PATH_PRODUCT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^/product/([A-Za-z0-9]+)/([0-9]+)(?:\.html)?/?$").expect("compiles")
});
static CSSBUY_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^/item-(micro-|1688-)?([0-9]+)\.html$").expect("compiles"));

const KEEP_PARAMS: [&str; 3] = ["id", "itemid", "userid"];
const TRAILING_PUNCT: &str = ".,;:!?)]}>，。！？；：（）】》「」『』、“”’‘";

fn agent_platform(name: &str) -> &'static str {
    match name {
        "weidian" | "wd" => "weidian",
        "taobao" | "tb" | "tao" => "taobao",
        "tmall" => "tmall",
        "1688" | "ali_1688" | "alibaba" | "ali" => "1688",
        _ => "",
    }
}

fn path_channel(agent: &str, code: &str) -> Option<&'static str> {
    let table: &[(&str, &str)] = match agent {
        "hoobuy.com" | "vigorbuy.com" => &[("0", "1688"), ("1", "taobao"), ("2", "weidian")],
        "usfans.com" => &[("1", "1688"), ("2", "taobao"), ("3", "weidian")],
        "litbuy.com" | "hipobuy.com" | "gtbuy.com" => {
            &[("0", "1688"), ("1", "taobao"), ("weidian", "weidian")]
        }
        _ => return None,
    };
    Some(
        table
            .iter()
            .find(|(k, _)| *k == code)
            .map_or("", |(_, v)| v),
    )
}

const WRAPPED_AGENTS: [&str; 9] = [
    "kakobuy.com",
    "superbuy.com",
    "sugargoo.com",
    "loongbuy.com",
    "pikobuy.com",
    "itaobuy.com",
    "bbdbuy.com",
    "allchinabuy.com",
    "pandabuy.com",
];

fn agent_query(agent: &str) -> Option<(&'static str, &'static str)> {
    Some(match agent {
        "oopbuy.com" | "ootdbuy.com" => ("id", "channel"),
        "mulebuy.com" | "joyagoo.com" | "cnfans.com" | "lovegobuy.com" => ("id", "shop_type"),
        "cnshopper.com" => ("keyword", "platform"),
        "alleurobuy.com" => ("id", "type"),
        "rizzitgo.com" => ("goodsid", "source"),
        _ => return None,
    })
}

const ID_PARAMS: [&str; 4] = ["id", "itemid", "goodsid", "keyword"];
const CHANNEL_PARAMS: [&str; 5] = ["channel", "shop_type", "source", "platform", "type"];
const WRAPPED_URL_PARAMS: [&str; 4] = ["url", "productlink", "producturl", "link"];

/// `urlparse(f"https://{host}/").hostname.casefold()` for a matched host.
fn hostname(host: &str) -> Option<String> {
    let parsed = pyurl::urlsplit(&format!("https://{host}/")).ok()?;
    let name = parsed.hostname()?;
    Some(name.to_lowercase().replace('\u{17f}', "s"))
}

fn labels_end_with(name: &str, suffix: &[&str]) -> bool {
    let labels: Vec<&str> = name.split('.').collect();
    labels.len() >= suffix.len() && labels[labels.len() - suffix.len()..] == *suffix
}

/// The marketplace a host belongs to, or "other".
pub fn platform(host: &str) -> &'static str {
    let Some(name) = hostname(host) else {
        return "other";
    };
    for (platform, _) in ITEM_URLS {
        if labels_end_with(&name, &[platform, "com"]) {
            return platform;
        }
    }
    "other"
}

fn agent(host: &str) -> Option<&'static str> {
    let name = hostname(host)?;
    AGENT_HOSTS
        .iter()
        .copied()
        .find(|a| labels_end_with(&name, &a.split('.').collect::<Vec<_>>()))
}

fn strip_fragment(rest: &str) -> &str {
    rest.split('#').next().unwrap_or("")
}

fn is_invisible(c: char) -> bool {
    matches!(
        c,
        '\x00'..='\x1f' | '\x7f' | '\u{feff}' | '\u{200b}' | '\u{200c}' | '\u{200d}' | '\u{2060}'
    )
}

fn normalize(host: &str, rest: &str) -> String {
    let rest = strip_fragment(if rest.is_empty() { "/" } else { rest });
    let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
    let kept: Vec<&str> = query
        .split('&')
        .filter(|pair| {
            !pair.is_empty()
                && KEEP_PARAMS
                    .contains(&pair.split('=').next().unwrap_or("").to_lowercase().as_str())
        })
        .collect();
    let mut url = format!("https://{host}{}", if path.is_empty() { "/" } else { path });
    if !kept.is_empty() {
        url.push('?');
        url.push_str(&kept.join("&"));
    }
    url
}

/// The marketplace link for a bare item id, or None for anything malformed.
pub fn item_link(platform: &str, item_id: &str) -> Option<Link> {
    let template = ITEM_URLS.iter().find(|(p, _)| *p == platform)?.1;
    let ok = (1..=24).contains(&item_id.len()) && item_id.bytes().all(|b| b.is_ascii_digit());
    ok.then(|| Link::new(platform, template.replace("{id}", item_id)))
}

fn unwrap_agent(agent: &str, rest: &str, depth: usize) -> Option<Link> {
    let rest = strip_fragment(if rest.is_empty() { "/" } else { rest });
    let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
    let mut values: HashMap<String, BTreeSet<String>> = HashMap::new();
    for (key, value) in pyurl::parse_qsl(query) {
        if !value.is_empty() {
            values.entry(key.to_lowercase()).or_default().insert(value);
        }
    }
    let ambiguous = |keys: &[&str]| {
        keys.iter()
            .any(|k| values.get(*k).is_some_and(|v| v.len() > 1))
    };
    let param = |key: &str| values.get(key).and_then(|v| v.iter().next()).cloned();
    if WRAPPED_AGENTS.contains(&agent) {
        if ambiguous(&WRAPPED_URL_PARAMS) {
            return None;
        }
        for key in WRAPPED_URL_PARAMS {
            if let Some(wrapped) = param(key).filter(|w| !w.is_empty()) {
                return wrapped_link(&wrapped, depth + 1);
            }
        }
        return None;
    }
    if let Some((id_key, channel_key)) = agent_query(agent) {
        let ids: HashSet<String> = ID_PARAMS.iter().filter_map(|k| param(k)).collect();
        let channels: HashSet<String> = CHANNEL_PARAMS
            .iter()
            .filter_map(|k| param(k))
            .map(|c| c.to_lowercase())
            .collect();
        let all: Vec<&str> = ID_PARAMS
            .iter()
            .chain(CHANNEL_PARAMS.iter())
            .copied()
            .collect();
        if ambiguous(&all) || ids.len() > 1 || channels.len() > 1 {
            return None;
        }
        let (Some(id), Some(channel)) = (param(id_key), param(channel_key)) else {
            return None;
        };
        let channel = channel.to_lowercase();
        let platform = if agent == "rizzitgo.com" {
            match channel.as_str() {
                "1" => "taobao",
                "2" => "1688",
                "3" => "weidian",
                _ => "",
            }
        } else {
            agent_platform(&channel)
        };
        return item_link(platform, &id);
    }
    if path_channel(agent, "").is_some()
        && let Some(caps) = PATH_PRODUCT_RE.captures(path)
    {
        let platform = path_channel(agent, &caps[1].to_lowercase()).unwrap_or("");
        return item_link(platform, &caps[2]);
    }
    if agent == "cssbuy.com"
        && let Some(caps) = CSSBUY_RE.captures(path)
    {
        let platform = match caps.get(1).map(|m| m.as_str()) {
            Some("micro-") => "weidian",
            Some("1688-") => "1688",
            _ => "taobao",
        };
        return item_link(platform, &caps[2]);
    }
    None
}

fn wrapped_link(url_text: &str, depth: usize) -> Option<Link> {
    let caps = URL_FULL_RE.captures(pystr::strip(url_text)).ok()??;
    let host = caps.name("host")?.as_str().to_lowercase();
    let cleaned = clean_rest(caps.name("rest").map(|m| m.as_str()));
    let rest = strip_fragment(&cleaned);
    let platform = platform(&host);
    if platform != "other" {
        return Some(Link::new(platform, normalize(&host, rest)));
    }
    match agent(&host) {
        Some(agent) if depth < 2 => unwrap_agent(agent, rest, depth),
        _ => None,
    }
}

/// The path and query of a candidate, ended at the first invisible or
/// non-ASCII character and shorn of trailing prose punctuation.
fn clean_rest(rest: Option<&str>) -> String {
    let rest = rest.unwrap_or("");
    let rest = rest.split(is_invisible).next().unwrap_or("");
    let rest = rest
        .split(|c: char| !('\x21'..='\x7e').contains(&c))
        .next()
        .unwrap_or("");
    rest.trim_end_matches(|c: char| TRAILING_PUNCT.contains(c))
        .to_string()
}

pub fn extract_links(text: &str) -> Vec<Link> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut links = Vec::new();
    for caps in URL_RE.captures_iter(text) {
        let Ok(caps) = caps else { break };
        let Some(host_match) = caps.name("host") else {
            continue;
        };
        let host = host_match.as_str().to_lowercase();
        let rest = clean_rest(caps.name("rest").map(|m| m.as_str()));
        let platform = platform(&host);
        let url = if platform == "other" {
            if let Some(agent) = agent(&host) {
                let Some(link) = unwrap_agent(agent, &rest, 0) else {
                    continue;
                };
                if seen.insert(link.url.clone()) {
                    links.push(link);
                }
                continue;
            }
            if !SHORTLINK_HOSTS.contains(&host.as_str()) {
                continue;
            }
            format!("https://{host}{}", strip_fragment(&rest))
        } else {
            normalize(&host, &rest)
        };
        if seen.insert(url.clone()) {
            links.push(Link::new(platform, url));
        }
    }
    links
}
