//! `urllib.parse` as Python 3.15 implements it: `urlsplit` (including its
//! ValueError cases), netloc accessors, `urljoin`, `quote`, `unquote` and
//! `parse_qsl`.

use unicode_normalization::UnicodeNormalization;

use crate::ipaddr;
use crate::pystr::repr;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Split {
    pub scheme: String,
    pub netloc: Option<String>,
    pub path: String,
    pub query: Option<String>,
    pub fragment: Option<String>,
}

const SCHEME_CHARS: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789+-.";

fn check_bracketed_host(hostname: &str) -> Result<(), String> {
    if hostname.starts_with(['v', 'V']) {
        let rest = &hostname[1..];
        let ok = match rest.split_once('.') {
            Some((hex, tail)) => {
                !hex.is_empty()
                    && hex.bytes().all(|b| b.is_ascii_hexdigit())
                    && !tail.is_empty()
                    && !tail.contains('\n')
            }
            None => false,
        };
        if !ok {
            return Err("IPvFuture address is invalid".into());
        }
        return Ok(());
    }
    match ipaddr::ip_address(hostname)? {
        (ipaddr::Ip::V4(_), _) => Err("An IPv4 address cannot be in brackets".into()),
        _ => Ok(()),
    }
}

fn check_bracketed_netloc(netloc: &str) -> Result<(), String> {
    let host_port = netloc.rsplit_once('@').map_or(netloc, |(_, h)| h);
    let hostname = match host_port.split_once('[') {
        Some((before, bracketed)) => {
            if !before.is_empty() {
                return Err("Invalid IPv6 URL".into());
            }
            let (hostname, port) = bracketed.split_once(']').unwrap_or((bracketed, ""));
            if !port.is_empty() && !port.starts_with(':') {
                return Err("Invalid IPv6 URL".into());
            }
            hostname
        }
        None => host_port.split_once(':').map_or(host_port, |(h, _)| h),
    };
    check_bracketed_host(hostname)
}

fn check_netloc(netloc: &str) -> Result<(), String> {
    if netloc.is_empty() || netloc.is_ascii() {
        return Ok(());
    }
    let n: String = netloc
        .chars()
        .filter(|c| !matches!(c, '@' | ':' | '#' | '?'))
        .collect();
    let normalised: String = n.nfkc().collect();
    if n == normalised {
        return Ok(());
    }
    if normalised.contains(['/', '?', '#', '@', ':']) {
        return Err(format!(
            "netloc '{netloc}' contains invalid characters under NFKC normalization"
        ));
    }
    Ok(())
}

/// `urlsplit(url)`; Err carries the ValueError text.
pub fn urlsplit(url: &str) -> Result<Split, String> {
    let url = url.trim_start_matches(|c: char| c <= ' ');
    let mut url: String = url
        .chars()
        .filter(|c| !matches!(c, '\t' | '\r' | '\n'))
        .collect();
    let mut split = Split::default();
    if let Some(i) = url.find(':')
        && i > 0
        && url.as_bytes()[0].is_ascii_alphabetic()
        && url[..i].chars().all(|c| SCHEME_CHARS.contains(c))
    {
        split.scheme = url[..i].to_ascii_lowercase();
        url = url[i + 1..].to_string();
    }
    if url.starts_with("//") {
        let rest = &url[2..];
        let delim = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        let netloc = rest[..delim].to_string();
        url = rest[delim..].to_string();
        let open = netloc.contains('[');
        let close = netloc.contains(']');
        if open != close {
            return Err("Invalid IPv6 URL".into());
        }
        if open && close {
            check_bracketed_netloc(&netloc)?;
        }
        split.netloc = Some(netloc);
    }
    if let Some((before, fragment)) = url.split_once('#') {
        split.fragment = Some(fragment.to_string());
        url = before.to_string();
    }
    if let Some((before, query)) = url.split_once('?') {
        split.query = Some(query.to_string());
        url = before.to_string();
    }
    if let Some(netloc) = &split.netloc {
        check_netloc(netloc)?;
    }
    split.path = url;
    Ok(split)
}

impl Split {
    pub fn netloc_str(&self) -> &str {
        self.netloc.as_deref().unwrap_or("")
    }

    pub fn query_str(&self) -> &str {
        self.query.as_deref().unwrap_or("")
    }

    fn userinfo(&self) -> (Option<String>, Option<String>) {
        let netloc = self.netloc_str();
        match netloc.rsplit_once('@') {
            Some((info, _)) => match info.split_once(':') {
                Some((user, password)) => (Some(user.into()), Some(password.into())),
                None => (Some(info.into()), None),
            },
            None => (None, None),
        }
    }

    pub fn username(&self) -> Option<String> {
        self.userinfo().0
    }

    pub fn password(&self) -> Option<String> {
        self.userinfo().1
    }

    fn hostinfo(&self) -> (String, Option<String>) {
        let netloc = self.netloc_str();
        let hostinfo = netloc.rsplit_once('@').map_or(netloc, |(_, h)| h);
        let (hostname, port) = match hostinfo.split_once('[') {
            Some((_, bracketed)) => {
                let (hostname, port) = bracketed.split_once(']').unwrap_or((bracketed, ""));
                let port = port.split_once(':').map_or("", |(_, p)| p);
                (hostname, port)
            }
            None => hostinfo.split_once(':').unwrap_or((hostinfo, "")),
        };
        let port = (!port.is_empty()).then(|| port.to_string());
        (hostname.to_string(), port)
    }

    /// `.hostname`: lowercased, zone id preserved.
    pub fn hostname(&self) -> Option<String> {
        let (hostname, _) = self.hostinfo();
        if hostname.is_empty() {
            return None;
        }
        Some(match hostname.split_once('%') {
            Some((host, zone)) => format!("{}%{zone}", host.to_lowercase()),
            None => hostname.to_lowercase(),
        })
    }

    /// `.port`; Err carries the ValueError text.
    pub fn port(&self) -> Result<Option<u32>, String> {
        let (_, port) = self.hostinfo();
        let Some(port) = port else { return Ok(None) };
        if !(port.is_ascii() && port.bytes().all(|b| b.is_ascii_digit())) {
            return Err(format!(
                "Port could not be cast to integer value as {}",
                repr(&port)
            ));
        }
        let trimmed = port.trim_start_matches('0');
        if trimmed.len() > 5 || trimmed.parse::<u32>().unwrap_or(0) > 65535 {
            return Err("Port out of range 0-65535".into());
        }
        Ok(Some(trimmed.parse().unwrap_or(0)))
    }
}

/// `_urlunsplit` with urlsplit's missing-as-empty components.
pub fn urlunsplit(scheme: &str, netloc: &str, path: &str, query: &str, fragment: &str) -> String {
    let uses_netloc = USES_NETLOC.contains(&scheme);
    let netloc = if !netloc.is_empty() {
        Some(netloc)
    } else if !scheme.is_empty() && uses_netloc && (path.is_empty() || path.starts_with('/')) {
        Some("")
    } else {
        None
    };
    unsplit_parts(
        (!scheme.is_empty()).then_some(scheme),
        netloc,
        path,
        (!query.is_empty()).then_some(query),
        (!fragment.is_empty()).then_some(fragment),
    )
}

fn unsplit_parts(
    scheme: Option<&str>,
    netloc: Option<&str>,
    path: &str,
    query: Option<&str>,
    fragment: Option<&str>,
) -> String {
    let mut url = path.to_string();
    if let Some(netloc) = netloc {
        if !url.is_empty() && !url.starts_with('/') {
            url.insert(0, '/');
        }
        url = format!("//{netloc}{url}");
    } else if url.starts_with("//") {
        url = format!("//{url}");
    }
    if let Some(scheme) = scheme
        && !scheme.is_empty()
    {
        url = format!("{scheme}:{url}");
    }
    if let Some(query) = query {
        url = format!("{url}?{query}");
    }
    if let Some(fragment) = fragment {
        url = format!("{url}#{fragment}");
    }
    url
}

const USES_RELATIVE: &[&str] = &[
    "", "ftp", "http", "gopher", "nntp", "imap", "wais", "file", "https", "shttp", "mms",
    "prospero", "rtsp", "rtsps", "rtspu", "sftp", "svn", "svn+ssh", "ws", "wss",
];
const USES_NETLOC: &[&str] = &[
    "",
    "ftp",
    "http",
    "gopher",
    "nntp",
    "telnet",
    "imap",
    "wais",
    "file",
    "mms",
    "https",
    "shttp",
    "snews",
    "prospero",
    "rtsp",
    "rtsps",
    "rtspu",
    "rsync",
    "svn",
    "svn+ssh",
    "sftp",
    "nfs",
    "git",
    "git+ssh",
    "ws",
    "wss",
    "itms-services",
];

fn split_optional(url: &str) -> Result<(Option<String>, Split), String> {
    let split = urlsplit(url)?;
    let scheme = (!split.scheme.is_empty()).then(|| split.scheme.clone());
    Ok((scheme, split))
}

/// `urljoin(base, url)`.
pub fn urljoin(base: &str, url: &str) -> Result<String, String> {
    if base.is_empty() {
        return Ok(url.to_string());
    }
    if url.is_empty() {
        return Ok(base.to_string());
    }
    let (bscheme, b) = split_optional(base)?;
    let (scheme, u) = split_optional(url)?;
    let scheme = scheme.or(bscheme.clone());
    let scheme_str = scheme.clone().unwrap_or_default();
    if scheme != bscheme
        || (!scheme_str.is_empty() && !USES_RELATIVE.contains(&scheme_str.as_str()))
    {
        return Ok(url.to_string());
    }
    let mut netloc = u.netloc.clone();
    if scheme_str.is_empty() || USES_NETLOC.contains(&scheme_str.as_str()) {
        if u.netloc.as_deref().is_some_and(|n| !n.is_empty()) {
            return Ok(unsplit_parts(
                scheme.as_deref(),
                u.netloc.as_deref(),
                &u.path,
                u.query.as_deref(),
                u.fragment.as_deref(),
            ));
        }
        netloc = b.netloc.clone();
    }
    if u.path.is_empty() {
        let mut query = u.query.clone();
        let mut fragment = u.fragment.clone();
        if query.is_none() {
            query = b.query.clone();
            if fragment.is_none() {
                fragment = b.fragment.clone();
            }
        }
        return Ok(unsplit_parts(
            scheme.as_deref(),
            netloc.as_deref(),
            &b.path,
            query.as_deref(),
            fragment.as_deref(),
        ));
    }
    let mut base_parts: Vec<&str> = b.path.split('/').collect();
    if base_parts.last() != Some(&"") {
        base_parts.pop();
    }
    let segments: Vec<&str> = if u.path.starts_with('/') {
        u.path.split('/').collect()
    } else {
        let mut all: Vec<&str> = base_parts;
        all.extend(u.path.split('/'));
        if all.len() > 2 {
            let last = all.len() - 1;
            let middle: Vec<&str> = all[1..last]
                .iter()
                .copied()
                .filter(|s| !s.is_empty())
                .collect();
            let mut rebuilt = vec![all[0]];
            rebuilt.extend(middle);
            rebuilt.push(all[last]);
            all = rebuilt;
        }
        all
    };
    let mut resolved: Vec<&str> = Vec::new();
    for seg in &segments {
        match *seg {
            ".." => {
                resolved.pop();
            }
            "." => {}
            other => resolved.push(other),
        }
    }
    if matches!(segments.last(), Some(&".") | Some(&"..")) {
        resolved.push("");
    }
    let joined = resolved.join("/");
    let path = if joined.is_empty() {
        "/".to_string()
    } else {
        joined
    };
    Ok(unsplit_parts(
        scheme.as_deref(),
        netloc.as_deref(),
        &path,
        u.query.as_deref(),
        u.fragment.as_deref(),
    ))
}

fn unquote_bytes(ascii: &str) -> Vec<u8> {
    let bytes = ascii.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && bytes[i + 1].is_ascii_hexdigit()
            && bytes[i + 2].is_ascii_hexdigit()
        {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("00");
            out.push(u8::from_str_radix(hex, 16).unwrap_or(0));
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    out
}

/// `unquote(s)`; with `strict`, invalid UTF-8 is an error (errors="strict").
pub fn unquote_with(s: &str, strict: bool) -> Result<String, String> {
    if !s.contains('%') {
        return Ok(s.to_string());
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while !rest.is_empty() {
        let ascii_len = rest.bytes().take_while(u8::is_ascii).count();
        if ascii_len == 0 {
            let non_ascii = rest
                .char_indices()
                .find(|(_, c)| c.is_ascii())
                .map_or(rest.len(), |(i, _)| i);
            out.push_str(&rest[..non_ascii]);
            rest = &rest[non_ascii..];
            continue;
        }
        let bytes = unquote_bytes(&rest[..ascii_len]);
        if strict {
            let text = crate::pyerr::decode_utf8(&bytes).map_err(|f| f.message)?;
            out.push_str(&text);
        } else {
            out.push_str(&String::from_utf8_lossy(&bytes));
        }
        rest = &rest[ascii_len..];
    }
    Ok(out)
}

pub fn unquote(s: &str) -> String {
    unquote_with(s, false).unwrap_or_default()
}

/// `quote(s, safe=safe)`.
pub fn quote(s: &str, safe: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        let c = b as char;
        if b.is_ascii_alphanumeric() || "_.-~".contains(c) || (b < 128 && safe.contains(c)) {
            out.push(c);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// `parse_qsl(qs)` with default arguments.
pub fn parse_qsl(qs: &str) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    for item in qs.split('&') {
        if item.is_empty() {
            continue;
        }
        let (name, value) = item.split_once('=').unwrap_or((item, ""));
        if value.is_empty() {
            continue;
        }
        pairs.push((
            unquote(&name.replace('+', " ")),
            unquote(&value.replace('+', " ")),
        ));
    }
    pairs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_and_accessors() {
        let s = urlsplit("HTTPS://User:pw@Example.COM:8443/a?b=1#c").unwrap();
        assert_eq!(s.scheme, "https");
        assert_eq!(s.hostname().as_deref(), Some("example.com"));
        assert_eq!(s.port(), Ok(Some(8443)));
        assert_eq!(s.username().as_deref(), Some("User"));
        assert!(urlsplit("https://[::1").is_err());
        assert!(urlsplit("https://[abc]/").is_err());
        assert!(urlsplit("https://[::1]:80]/x").unwrap().port().is_err());
        assert!(urlsplit("https://a/\u{ff0f}b").is_ok());
        assert!(urlsplit("https://a\u{ff0f}b/").is_err());
        assert_eq!(
            urlsplit("https://a:bad/").unwrap().port(),
            Err("Port could not be cast to integer value as 'bad'".into())
        );
    }

    #[test]
    fn join_and_quote() {
        assert_eq!(urljoin("https://h/a/b", "/x").unwrap(), "https://h/x");
        assert_eq!(urljoin("https://h/a/b", "c/../d").unwrap(), "https://h/a/d");
        assert_eq!(urljoin("https://h/a/b", "//o/p").unwrap(), "https://o/p");
        assert_eq!(
            urljoin("https://example.invalid/", "/./a/../b").unwrap(),
            "https://example.invalid/b"
        );
        assert_eq!(quote("/a b/é", "/"), "/a%20b/%C3%A9");
        assert_eq!(unquote("%e2%82"), "\u{fffd}");
        assert_eq!(
            parse_qsl("a=1&b&c=&d=%41+b"),
            vec![("a".into(), "1".into()), ("d".into(), "A b".into())]
        );
    }
}
