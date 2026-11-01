//! A port of what `BeautifulSoup(html, "html.parser")` does with a page:
//! CPython 3.15's `html.parser` tokenizer (with `convert_charrefs=False`, as
//! bs4 sets it), bs4's tree construction, `get_text`, and the few CSS
//! selectors the parsers use.

use crate::pystr;
use crate::pytables;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StrKind {
    Navigable,
    CData,
    Comment,
    Doctype,
    Declaration,
    Pi,
    Script,
    Stylesheet,
    Template,
    RubyText,
    RubyParen,
}

#[derive(Debug)]
enum Kind {
    Element {
        name: String,
        attrs: Vec<(String, String)>,
    },
    Text {
        text: String,
        kind: StrKind,
    },
}

#[derive(Debug)]
struct Node {
    kind: Kind,
    children: Vec<usize>,
}

pub struct Document {
    nodes: Vec<Node>,
}

pub type NodeId = usize;

const VOID: [&str; 24] = [
    "area", "base", "basefont", "bgsound", "br", "col", "command", "embed", "frame", "hr", "image",
    "img", "input", "isindex", "keygen", "link", "menuitem", "meta", "nextid", "param", "source",
    "spacer", "track", "wbr",
];
const CDATA_CONTENT: [&str; 6] = ["script", "style", "xmp", "iframe", "noembed", "noframes"];
const RCDATA_CONTENT: [&str; 2] = ["textarea", "title"];

fn string_container(name: &str) -> Option<StrKind> {
    Some(match name {
        "rt" => StrKind::RubyText,
        "rp" => StrKind::RubyParen,
        "style" => StrKind::Stylesheet,
        "script" => StrKind::Script,
        "template" => StrKind::Template,
        _ => return None,
    })
}

fn is_tag_ws(b: u8) -> bool {
    matches!(b, b'\t' | b'\n' | b'\r' | b'\x0c' | b' ')
}

struct Builder {
    nodes: Vec<Node>,
    stack: Vec<usize>,
    current_data: Vec<String>,
    preserve_stack: Vec<usize>,
    container_stack: Vec<usize>,
    already_closed_empty: Vec<String>,
}

impl Builder {
    fn name(&self, id: usize) -> &str {
        match &self.nodes[id].kind {
            Kind::Element { name, .. } => name,
            Kind::Text { .. } => "",
        }
    }

    fn current(&self) -> usize {
        *self.stack.last().unwrap_or(&0)
    }

    fn end_data(&mut self, class: Option<StrKind>) {
        if self.current_data.is_empty() {
            return;
        }
        let mut data = self.current_data.concat();
        self.current_data.clear();
        if self.preserve_stack.is_empty() && data.chars().all(|c| " \n\t\x0c\r".contains(c)) {
            data = if data.contains('\n') {
                "\n".into()
            } else {
                " ".into()
            };
        }
        let mut kind = class.unwrap_or(StrKind::Navigable);
        if kind == StrKind::Navigable
            && let Some(&top) = self.container_stack.last()
            && let Some(k) = string_container(self.name(top))
        {
            kind = k;
        }
        let id = self.nodes.len();
        self.nodes.push(Node {
            kind: Kind::Text { text: data, kind },
            children: Vec::new(),
        });
        let parent = self.current();
        self.nodes[parent].children.push(id);
    }

    fn push_tag(&mut self, name: String, attrs: Vec<(String, String)>) -> usize {
        self.end_data(None);
        let id = self.nodes.len();
        let preserve = matches!(name.as_str(), "pre" | "textarea");
        let container = string_container(&name).is_some();
        self.nodes.push(Node {
            kind: Kind::Element { name, attrs },
            children: Vec::new(),
        });
        let parent = self.current();
        self.nodes[parent].children.push(id);
        self.stack.push(id);
        if preserve {
            self.preserve_stack.push(id);
        }
        if container {
            self.container_stack.push(id);
        }
        id
    }

    fn pop_tag(&mut self) {
        if self.stack.len() <= 1 {
            return;
        }
        let Some(id) = self.stack.pop() else { return };
        if self.preserve_stack.last() == Some(&id) {
            self.preserve_stack.pop();
        }
        if self.container_stack.last() == Some(&id) {
            self.container_stack.pop();
        }
    }

    fn pop_to_tag(&mut self, name: &str) {
        let open = self.stack[1..]
            .iter()
            .rev()
            .any(|&id| self.name(id) == name);
        if !open {
            return;
        }
        while self.stack.len() > 1 {
            let top = self.current();
            let matched = self.name(top) == name;
            self.pop_tag();
            if matched {
                break;
            }
        }
    }

    fn soup_end_tag(&mut self, name: &str) {
        self.end_data(None);
        self.pop_to_tag(name);
    }

    fn start_tag(&mut self, tag: &str, raw: Vec<(String, Option<String>)>, handle_empty: bool) {
        let mut attrs: Vec<(String, String)> = Vec::new();
        for (key, value) in raw {
            let value = value.unwrap_or_default();
            match attrs.iter_mut().find(|(k, _)| *k == key) {
                Some(entry) => entry.1 = value,
                None => attrs.push((key, value)),
            }
        }
        self.push_tag(tag.to_string(), attrs);
        if handle_empty && VOID.contains(&tag) {
            self.end_tag(tag, false);
            self.already_closed_empty.push(tag.to_string());
        }
    }

    fn end_tag(&mut self, tag: &str, check_already_closed: bool) {
        if check_already_closed
            && let Some(pos) = self.already_closed_empty.iter().position(|t| t == tag)
        {
            self.already_closed_empty.remove(pos);
        } else {
            self.soup_end_tag(tag);
        }
    }

    fn data(&mut self, data: &str) {
        self.current_data.push(data.to_string());
    }

    fn special(&mut self, data: &str, kind: StrKind) {
        self.end_data(None);
        self.data(data);
        self.end_data(Some(kind));
    }

    fn charref(&mut self, name: &str) {
        let (hex, digits) = match name.strip_prefix(['x', 'X']) {
            Some(rest) => (true, rest),
            None => (false, name),
        };
        let radix = if hex { 16 } else { 10 };
        let whole = !digits.is_empty()
            && digits
                .chars()
                .all(|c| c.is_ascii() && (c as u8 as char).is_digit(radix));
        let (number, extra) = if whole {
            (Some(big_number(digits, radix)), String::new())
        } else {
            // _DECIMAL/_HEX_REFERENCE_WITH_FOLLOWING_DATA: "^([0-9]+)(.*)",
            // where "." stops at a newline and hex digits are lowercase only.
            let lead = digits
                .bytes()
                .take_while(|b| {
                    if hex {
                        b.is_ascii_digit() || (b'a'..=b'f').contains(b)
                    } else {
                        b.is_ascii_digit()
                    }
                })
                .count();
            if lead == 0 {
                (None, digits.to_string())
            } else {
                let rest = &digits[lead..];
                let line = rest.split('\n').next().unwrap_or("");
                (Some(big_number(&digits[..lead], radix)), line.to_string())
            }
        };
        let text = match number {
            None => String::new(),
            Some(n) => numeric_character_reference(n),
        };
        self.data(&text);
        self.data(&extra);
    }

    fn entityref(&mut self, name: &str) {
        match lookup(pytables::BS4_ENTITIES, name) {
            Some(value) => self.data(value),
            None => self.data(&format!("&{name}")),
        }
    }
}

/// An integer of arbitrary size, saturated past the Unicode range.
fn big_number(digits: &str, radix: u32) -> u64 {
    let mut value: u64 = 0;
    for c in digits.chars() {
        let d = c.to_digit(radix).unwrap_or(0);
        value = value
            .saturating_mul(u64::from(radix))
            .saturating_add(u64::from(d));
    }
    value
}

fn lookup(table: &'static [(&'static str, &'static str)], key: &str) -> Option<&'static str> {
    table
        .binary_search_by(|(k, _)| (*k).cmp(key))
        .ok()
        .map(|i| table[i].1)
}

/// `UnicodeDammit.numeric_character_reference`.
fn numeric_character_reference(n: u64) -> String {
    if n == 0 || n > 0x10ffff || (0xd800..=0xdfff).contains(&n) {
        return "\u{fffd}".into();
    }
    let n32 = n as u32;
    if (0xfdd0..=0xfdef).contains(&n32) || pytables::NONCHARACTERS.contains(&n32) {
        return char::from_u32(n32).map(String::from).unwrap_or_default();
    }
    if (0x80..=0x9f).contains(&n32)
        && let Some(&(_, c)) = pytables::WINDOWS_1252_C1.iter().find(|(k, _)| *k == n32)
    {
        return c.to_string();
    }
    char::from_u32(n32).map_or_else(|| "\u{fffd}".into(), String::from)
}

/// `html.unescape` applied to one matched reference.
fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let b = s.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    let mut last = 0;
    while i < b.len() {
        if b[i] != b'&' {
            i += 1;
            continue;
        }
        // _charref = &(#[0-9]+;?|#[xX][0-9a-fA-F]+;?|[^\t\n\f <&#;]{1,32};?)
        let start = i + 1;
        let mut end = start;
        let mut replacement = None;
        if b.get(start) == Some(&b'#') {
            let hex = matches!(b.get(start + 1), Some(b'x' | b'X'));
            let digits_start = if hex { start + 2 } else { start + 1 };
            let mut j = digits_start;
            while j < b.len()
                && (if hex {
                    b[j].is_ascii_hexdigit()
                } else {
                    b[j].is_ascii_digit()
                })
            {
                j += 1;
            }
            if j > digits_start {
                let num = big_number(&s[digits_start..j], if hex { 16 } else { 10 });
                end = if b.get(j) == Some(&b';') { j + 1 } else { j };
                replacement = Some(
                    if let Some(&(_, v)) = pytables::INVALID_CHARREFS
                        .iter()
                        .find(|(k, _)| u64::from(*k) == num)
                    {
                        v.to_string()
                    } else if (0xd800..=0xdfff).contains(&num) || num > 0x10ffff {
                        "\u{fffd}".to_string()
                    } else if pytables::INVALID_CODEPOINTS.contains(&(num as u32)) {
                        String::new()
                    } else {
                        char::from_u32(num as u32)
                            .map(String::from)
                            .unwrap_or_default()
                    },
                );
            }
        } else {
            let mut count = 0;
            let mut j = start;
            for c in s[start..].chars() {
                if count == 32 || matches!(c, '\t' | '\n' | '\x0c' | ' ' | '<' | '&' | '#' | ';') {
                    break;
                }
                count += 1;
                j += c.len_utf8();
            }
            if count > 0 {
                end = if b.get(j) == Some(&b';') { j + 1 } else { j };
                let name = &s[start..end];
                replacement = Some(match lookup(pytables::HTML5, name) {
                    Some(v) => v.to_string(),
                    None => {
                        let chars: Vec<(usize, char)> = name.char_indices().collect();
                        let mut found = None;
                        for x in (2..chars.len()).rev() {
                            let cut = chars[x].0;
                            if let Some(v) = lookup(pytables::HTML5, &name[..cut]) {
                                found = Some(format!("{v}{}", &name[cut..]));
                                break;
                            }
                        }
                        found.unwrap_or_else(|| format!("&{name}"))
                    }
                });
            }
        }
        match replacement {
            Some(text) => {
                out.push_str(&s[last..i]);
                out.push_str(&text);
                i = end;
                last = end;
            }
            None => i += 1,
        }
    }
    out.push_str(&s[last..]);
    out
}

/// `_unescape_attrvalue`: attr_charref.sub(_replace_attr_charref, s).
fn unescape_attr(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    let mut last = 0;
    while i < b.len() {
        if b[i] != b'&' {
            i += 1;
            continue;
        }
        // &(#[0-9]+|#[xX][0-9a-fA-F]+|[a-zA-Z][a-zA-Z0-9]*)[;=]?
        let mut j = i + 1;
        let numeric;
        if b.get(j) == Some(&b'#') {
            numeric = true;
            let digits_at = j + 1;
            let mut k = digits_at;
            while k < b.len() && b[k].is_ascii_digit() {
                k += 1;
            }
            if k > digits_at {
                j = k;
            } else if matches!(b.get(digits_at), Some(b'x' | b'X')) {
                let mut k = digits_at + 1;
                while k < b.len() && b[k].is_ascii_hexdigit() {
                    k += 1;
                }
                if k == digits_at + 1 {
                    i += 1;
                    continue;
                }
                j = k;
            } else {
                i += 1;
                continue;
            }
        } else if b.get(j).is_some_and(u8::is_ascii_alphabetic) {
            numeric = false;
            j += 1;
            while j < b.len() && b[j].is_ascii_alphanumeric() {
                j += 1;
            }
        } else {
            i += 1;
            continue;
        }
        if matches!(b.get(j), Some(b';' | b'=')) {
            j += 1;
        }
        let reference = &s[i..j];
        let exact_named =
            !reference.ends_with('=') && lookup(pytables::HTML5, &reference[1..]).is_some();
        let replaced = if numeric || exact_named {
            unescape(reference)
        } else {
            reference.to_string()
        };
        out.push_str(&s[last..i]);
        out.push_str(&replaced);
        i = j;
        last = j;
    }
    out.push_str(&s[last..]);
    out
}

struct Tokenizer<'a> {
    raw: &'a str,
    b: &'a [u8],
    cdata_elem: Option<String>,
    escapable: bool,
}

/// `[a-zA-Z][^\t\n\r\f />]*` followed by `(?:[\t\n\r\f ]|/(?!>))*`.
fn tagfind(b: &[u8], p: usize) -> Option<(usize, usize, usize)> {
    if !b.get(p).is_some_and(u8::is_ascii_alphabetic) {
        return None;
    }
    let mut j = p + 1;
    while j < b.len() && !is_tag_ws(b[j]) && b[j] != b'/' && b[j] != b'>' {
        j += 1;
    }
    let name_end = j;
    while j < b.len() && (is_tag_ws(b[j]) || (b[j] == b'/' && b.get(j + 1) != Some(&b'>'))) {
        j += 1;
    }
    Some((p, name_end, j))
}

/// `[\t\n\r\f ]*=[\t\n\r\f ]*(value)`, with the regex's backtracking: an
/// unclosed quote gives back the spaces after `=` and takes an empty value.
fn value_indicator(b: &[u8], p: usize) -> Option<(usize, usize, usize)> {
    let mut j = p;
    while j < b.len() && is_tag_ws(b[j]) {
        j += 1;
    }
    if b.get(j) != Some(&b'=') {
        return None;
    }
    let after_eq = j + 1;
    let mut spaces_end = after_eq;
    while spaces_end < b.len() && is_tag_ws(b[spaces_end]) {
        spaces_end += 1;
    }
    let mut start = spaces_end;
    loop {
        match b.get(start) {
            Some(&q @ (b'\'' | b'"')) => {
                if let Some(close) = b[start + 1..].iter().position(|&c| c == q) {
                    let end = start + 1 + close + 1;
                    return Some((start, end, end));
                }
            }
            _ => {
                let mut end = start;
                while end < b.len() && b[end] != b'>' && !is_tag_ws(b[end]) {
                    end += 1;
                }
                return Some((start, end, end));
            }
        }
        if start == after_eq {
            return None;
        }
        start -= 1;
    }
}

fn attr_name_at(b: &[u8], k: usize) -> Option<usize> {
    let before_ok = k > 0
        && matches!(
            b[k - 1],
            b'\'' | b'"' | b'\t' | b'\n' | b'\r' | b'\x0c' | b' ' | b'/'
        );
    let first = *b.get(k)?;
    if !before_ok || is_tag_ws(first) || first == b'/' || first == b'>' {
        return None;
    }
    let mut j = k + 1;
    while j < b.len() && !is_tag_ws(b[j]) && b[j] != b'/' && b[j] != b'=' && b[j] != b'>' {
        j += 1;
    }
    Some(j)
}

/// `locatetagend.match(rawdata, p).end()`.
fn locatetagend(b: &[u8], p: usize) -> usize {
    let mut j = p + 1;
    while j < b.len() && !is_tag_ws(b[j]) && b[j] != b'/' && b[j] != b'>' {
        j += 1;
    }
    while j < b.len() && (is_tag_ws(b[j]) || b[j] == b'/') {
        j += 1;
    }
    while let Some(name_end) = attr_name_at(b, j) {
        j = name_end;
        if let Some((_, _, end)) = value_indicator(b, j) {
            j = end;
        }
        while j < b.len() && (is_tag_ws(b[j]) || b[j] == b'/') {
            j += 1;
        }
    }
    if b.get(j) == Some(&b'>') {
        j += 1;
    }
    j
}

impl<'a> Tokenizer<'a> {
    fn starts(&self, i: usize, s: &str) -> bool {
        self.b[i..].starts_with(s.as_bytes())
    }

    fn find(&self, needle: &str, from: usize) -> Option<usize> {
        if from > self.b.len() {
            return None;
        }
        self.raw[from..].find(needle).map(|p| p + from)
    }

    fn interesting(&self, i: usize) -> Option<usize> {
        match &self.cdata_elem {
            None => self.b[i..]
                .iter()
                .position(|&c| c == b'&' || c == b'<')
                .map(|p| p + i),
            Some(elem) if elem == "plaintext" => None,
            Some(elem) => {
                let pattern = format!("</{elem}");
                let mut j = i;
                while j < self.b.len() {
                    if self.escapable && self.b[j] == b'&' {
                        return Some(j);
                    }
                    if self.b[j] == b'<'
                        && self.b.len() >= j + pattern.len()
                        && self.b[j..j + pattern.len()].eq_ignore_ascii_case(pattern.as_bytes())
                        && self
                            .b
                            .get(j + pattern.len())
                            .is_some_and(|&c| is_tag_ws(c) || c == b'/' || c == b'>')
                    {
                        return Some(j);
                    }
                    j += 1;
                }
                None
            }
        }
    }

    fn set_cdata_mode(&mut self, elem: &str, escapable: bool) {
        self.cdata_elem = Some(elem.to_lowercase());
        self.escapable = escapable;
    }

    fn clear_cdata_mode(&mut self) {
        self.cdata_elem = None;
        self.escapable = true;
    }

    fn parse_starttag(&mut self, i: usize, out: &mut Builder) -> Option<usize> {
        let endpos = locatetagend(self.b, i + 1);
        if self.b[endpos - 1] != b'>' {
            return None;
        }
        let (start, name_end, mut k) = tagfind(self.b, i + 1)?;
        let tag = self.raw[start..name_end].to_lowercase();
        let mut attrs: Vec<(String, Option<String>)> = Vec::new();
        while k < endpos {
            let Some(name_end) = attr_name_at(self.b, k) else {
                break;
            };
            let name = self.raw[k..name_end].to_lowercase();
            let mut j = name_end;
            let mut value = None;
            if let Some((vs, ve, end)) = value_indicator(self.b, j) {
                let mut v = &self.raw[vs..ve];
                if v.len() >= 2
                    && ((v.starts_with('\'') && v.ends_with('\''))
                        || (v.starts_with('"') && v.ends_with('"')))
                {
                    v = &v[1..v.len() - 1];
                } else if v == "'" || v == "\"" {
                    v = "";
                }
                value = Some(if v.is_empty() {
                    String::new()
                } else {
                    unescape_attr(v)
                });
                j = end;
            }
            while j < self.b.len()
                && (is_tag_ws(self.b[j]) || (self.b[j] == b'/' && self.b.get(j + 1) != Some(&b'>')))
            {
                j += 1;
            }
            attrs.push((name, value));
            k = j;
        }
        let end = pystr::strip(&self.raw[k..endpos]);
        if end != ">" && end != "/>" {
            out.data(&self.raw[i..endpos]);
            return Some(endpos);
        }
        if end.ends_with("/>") {
            out.start_tag(&tag, attrs, false);
            out.end_tag(&tag, false);
        } else {
            out.start_tag(&tag, attrs, true);
            if CDATA_CONTENT.contains(&tag.as_str()) || tag == "plaintext" {
                self.set_cdata_mode(&tag, false);
            } else if RCDATA_CONTENT.contains(&tag.as_str()) {
                self.set_cdata_mode(&tag, true);
            }
        }
        Some(endpos)
    }

    fn parse_bogus_comment(&self, i: usize, out: &mut Builder) -> Option<usize> {
        let pos = self.find(">", i + 2)?;
        out.special(&self.raw[i + 2..pos], StrKind::Comment);
        Some(pos + 1)
    }

    fn parse_endtag(&mut self, i: usize, out: &mut Builder) -> Option<usize> {
        self.find(">", i + 2)?;
        if !self.b.get(i + 2).is_some_and(u8::is_ascii_alphabetic) {
            if self.b.get(i + 2) == Some(&b'>') {
                return Some(i + 3);
            }
            return self.parse_bogus_comment(i, out);
        }
        let j = locatetagend(self.b, i + 2);
        if self.b[j - 1] != b'>' {
            return None;
        }
        let (start, name_end, _) = tagfind(self.b, i + 2)?;
        let tag = self.raw[start..name_end].to_lowercase();
        out.end_tag(&tag, true);
        self.clear_cdata_mode();
        Some(j)
    }

    fn parse_comment(&self, i: usize, out: &mut Builder) -> Option<usize> {
        let p = i + 4;
        // commentabruptclose: -?> at p
        if self.starts(p, ">") || self.starts(p, "->") {
            out.special("", StrKind::Comment);
            return Some(if self.starts(p, ">") { p + 1 } else { p + 2 });
        }
        let mut j = p;
        while let Some(found) = self.find("--", j) {
            if self.starts(found + 2, ">") {
                out.special(&self.raw[p..found], StrKind::Comment);
                return Some(found + 3);
            }
            if self.starts(found + 2, "!>") {
                out.special(&self.raw[p..found], StrKind::Comment);
                return Some(found + 4);
            }
            j = found + 1;
        }
        None
    }

    fn parse_html_declaration(&self, i: usize, out: &mut Builder) -> Option<usize> {
        if self.starts(i, "<!--") {
            return self.parse_comment(i, out);
        }
        if self.starts(i, "<![CDATA[") {
            let j = self.find("]]>", i + 9)?;
            out.special(&self.raw[i + 9..j], StrKind::CData);
            return Some(j + 3);
        }
        if self.b.len() >= i + 9 && self.b[i..i + 9].eq_ignore_ascii_case(b"<!doctype") {
            let gt = self.find(">", i + 9)?;
            out.special(&doctype_text(&self.raw[i + 2..gt]), StrKind::Doctype);
            return Some(gt + 1);
        }
        self.parse_bogus_comment(i, out)
    }

    fn parse_pi(&self, i: usize, out: &mut Builder) -> Option<usize> {
        let j = self.find(">", i + 2)?;
        out.special(&self.raw[i + 2..j], StrKind::Pi);
        Some(j + 1)
    }

    /// `goahead(end)` from `start`; returns where it stopped.
    fn goahead(&mut self, start: usize, end: bool, out: &mut Builder) -> usize {
        let n = self.b.len();
        let mut i = start;
        while i < n {
            let j = match self.interesting(i) {
                Some(j) => j,
                None => {
                    if self.cdata_elem.is_some() {
                        break;
                    }
                    n
                }
            };
            if i < j {
                out.data(&self.raw[i..j]);
            }
            i = j;
            if i == n {
                break;
            }
            if self.starts(i, "<") {
                let k = if self.b.get(i + 1).is_some_and(u8::is_ascii_alphabetic) {
                    self.parse_starttag(i, out)
                } else if self.starts(i, "</") {
                    self.parse_endtag(i, out)
                } else if self.starts(i, "<!--") {
                    self.parse_comment(i, out)
                } else if self.starts(i, "<?") {
                    self.parse_pi(i, out)
                } else if self.starts(i, "<!") {
                    self.parse_html_declaration(i, out)
                } else if i + 1 < n || end {
                    out.data("<");
                    Some(i + 1)
                } else {
                    break;
                };
                match k {
                    Some(k) => i = k,
                    None => {
                        if !end {
                            break;
                        }
                        if self.b.get(i + 1).is_some_and(u8::is_ascii_alphabetic) {
                        } else if self.starts(i, "</") {
                            if i + 2 == n {
                                out.data("</");
                            } else if self.b.get(i + 2).is_some_and(u8::is_ascii_alphabetic) {
                            } else {
                                out.special(&self.raw[i + 2..], StrKind::Comment);
                            }
                        } else if self.starts(i, "<!--") {
                            let mut j = n;
                            for suffix in ["--!", "--", "-"] {
                                if self.raw[i + 4..].ends_with(suffix) {
                                    j -= suffix.len();
                                    break;
                                }
                            }
                            out.special(&self.raw[i + 4..j.max(i + 4)], StrKind::Comment);
                        } else if self.starts(i, "<![CDATA[") {
                            out.special(&self.raw[i + 9..], StrKind::CData);
                        } else if self.b.len() >= i + 9
                            && self.b[i..i + 9].eq_ignore_ascii_case(b"<!doctype")
                        {
                            out.special(&doctype_text(&self.raw[i + 2..]), StrKind::Doctype);
                        } else if self.starts(i, "<!") {
                            out.special(&self.raw[i + 2..], StrKind::Comment);
                        } else if self.starts(i, "<?") {
                            out.special(&self.raw[i + 2..], StrKind::Pi);
                        }
                        i = n;
                    }
                }
            } else if self.starts(i, "&#") {
                if let Some((name_end, term)) = self.match_charref(i) {
                    out.charref(&self.raw[i + 2..name_end]);
                    i = if term == Some(b';') {
                        name_end + 1
                    } else {
                        name_end
                    };
                    continue;
                }
                let incomplete = match self.b.get(i + 2) {
                    Some(c) if c.is_ascii_digit() => true,
                    Some(b'x' | b'X') => self.b.get(i + 3).is_some_and(u8::is_ascii_hexdigit),
                    _ => false,
                };
                if incomplete {
                    if end {
                        out.charref(&self.raw[i + 2..]);
                        i = n;
                    }
                    break;
                } else if i + 3 < n {
                    out.data("&#");
                    i += 2;
                } else {
                    break;
                }
            } else if self.starts(i, "&") {
                if let Some((name_end, term)) = self.match_entityref(i) {
                    out.entityref(&self.raw[i + 1..name_end]);
                    i = if term == Some(b';') {
                        name_end + 1
                    } else {
                        name_end
                    };
                    continue;
                }
                let incomplete = self
                    .b
                    .get(i + 1)
                    .is_some_and(|c| c.is_ascii_alphabetic() || *c == b'#');
                if incomplete {
                    if end {
                        out.entityref(&self.raw[i + 1..]);
                        i = n;
                    }
                    break;
                } else if i + 1 < n {
                    out.data("&");
                    i += 1;
                } else {
                    break;
                }
            }
        }
        if end && i < n {
            out.data(&self.raw[i..n]);
            i = n;
        }
        i
    }

    /// `&#(?:[0-9]+|[xX][0-9a-fA-F]+)[^0-9a-fA-F]`: (end of name, terminator).
    fn match_charref(&self, i: usize) -> Option<(usize, Option<u8>)> {
        let b = self.b;
        let mut j = i + 2;
        let decimal_start = j;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        let try_end = |end: usize| -> Option<(usize, Option<u8>)> {
            match b.get(end) {
                Some(c) if !c.is_ascii_hexdigit() => Some((end, Some(*c))),
                _ => None,
            }
        };
        if j > decimal_start {
            // Backtracking cannot help: a shorter digit run ends on a digit.
            if let Some(found) = try_end(j) {
                return Some(found);
            }
        }
        if matches!(b.get(i + 2), Some(b'x' | b'X')) {
            let mut k = i + 3;
            while k < b.len() && b[k].is_ascii_hexdigit() {
                k += 1;
            }
            if k > i + 3 {
                return try_end(k);
            }
        }
        None
    }

    /// `&([a-zA-Z][-.a-zA-Z0-9]*)[^a-zA-Z0-9]`: (end of name, terminator).
    fn match_entityref(&self, i: usize) -> Option<(usize, Option<u8>)> {
        let b = self.b;
        if !b.get(i + 1).is_some_and(u8::is_ascii_alphabetic) {
            return None;
        }
        let mut j = i + 2;
        while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'-' || b[j] == b'.') {
            j += 1;
        }
        if let Some(&c) = b.get(j) {
            return Some((j, Some(c)));
        }
        // At the end of input, backtrack to the last '-' or '.' in the name.
        (i + 2..j)
            .rev()
            .find(|&k| b[k] == b'-' || b[k] == b'.')
            .map(|k| (k, Some(b[k])))
    }
}

fn doctype_text(decl: &str) -> String {
    // bs4: decl[len("DOCTYPE "):]
    decl.chars().skip(8).collect()
}

pub fn parse(html: &str) -> Document {
    let mut builder = Builder {
        nodes: vec![Node {
            kind: Kind::Element {
                name: "[document]".into(),
                attrs: Vec::new(),
            },
            children: Vec::new(),
        }],
        stack: vec![0],
        current_data: Vec::new(),
        preserve_stack: Vec::new(),
        container_stack: Vec::new(),
        already_closed_empty: Vec::new(),
    };
    let mut tokenizer = Tokenizer {
        raw: html,
        b: html.as_bytes(),
        cdata_elem: None,
        escapable: true,
    };
    let stopped = if html.is_empty() {
        0
    } else {
        tokenizer.goahead(0, false, &mut builder)
    };
    tokenizer.goahead(stopped, true, &mut builder);
    builder.end_data(None);
    Document {
        nodes: builder.nodes,
    }
}

/// One compound selector: optional tag, classes, attribute equalities.
#[derive(Debug, Default)]
struct Compound {
    tag: Option<String>,
    classes: Vec<String>,
    attrs: Vec<(String, String)>,
}

fn parse_selector(selector: &str) -> Vec<Compound> {
    selector
        .split(',')
        .map(|part| {
            let part = part.trim();
            let mut compound = Compound::default();
            let mut rest = part;
            let tag_len = rest.find(['.', '[']).unwrap_or(rest.len());
            if tag_len > 0 {
                compound.tag = Some(rest[..tag_len].to_lowercase());
            }
            rest = &rest[tag_len..];
            while !rest.is_empty() {
                if let Some(after) = rest.strip_prefix('.') {
                    let len = after.find(['.', '[']).unwrap_or(after.len());
                    compound.classes.push(after[..len].to_string());
                    rest = &after[len..];
                } else if let Some(after) = rest.strip_prefix('[') {
                    let close = after.find(']').unwrap_or(after.len());
                    let (name, value) = after[..close]
                        .split_once('=')
                        .unwrap_or((&after[..close], ""));
                    compound
                        .attrs
                        .push((name.to_lowercase(), value.trim_matches('"').to_string()));
                    rest = after.get(close + 1..).unwrap_or("");
                } else {
                    break;
                }
            }
            compound
        })
        .collect()
}

/// Python IGNORECASE equality for an attribute value against an ASCII
/// pattern, with `$` allowing one trailing newline.
fn type_value_matches(value: &str, pattern: &str) -> bool {
    let ci_eq = |value: &str| {
        let mut vc = value.chars();
        for p in pattern.chars() {
            let Some(v) = vc.next() else { return false };
            let ok = v == p
                || (v.is_ascii() && v.eq_ignore_ascii_case(&p))
                || matches!(
                    (p.to_ascii_lowercase(), v),
                    ('i', '\u{130}' | '\u{131}') | ('s', '\u{17f}') | ('k', '\u{212a}')
                );
            if !ok {
                return false;
            }
        }
        vc.next().is_none()
    };
    ci_eq(value) || value.strip_suffix('\n').is_some_and(ci_eq)
}

impl Document {
    pub fn root(&self) -> NodeId {
        0
    }

    fn element(&self, id: NodeId) -> Option<(&str, &[(String, String)])> {
        match &self.nodes[id].kind {
            Kind::Element { name, attrs } => Some((name, attrs)),
            Kind::Text { .. } => None,
        }
    }

    /// `tag.get(key)` when it is a plain string (class-like attributes are lists).
    pub fn attr(&self, id: NodeId, key: &str) -> Option<&str> {
        let (name, attrs) = self.element(id)?;
        if is_multi_valued(name, key) {
            return None;
        }
        attrs
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    fn classes(&self, id: NodeId) -> Vec<&str> {
        self.element(id)
            .and_then(|(_, attrs)| attrs.iter().find(|(k, _)| k == "class"))
            .map(|(_, v)| v.split(pystr::is_space).filter(|s| !s.is_empty()).collect())
            .unwrap_or_default()
    }

    fn matches(&self, id: NodeId, compound: &Compound) -> bool {
        let Some((name, attrs)) = self.element(id) else {
            return false;
        };
        if compound.tag.as_deref().is_some_and(|t| t != name) {
            return false;
        }
        let classes = self.classes(id);
        if !compound
            .classes
            .iter()
            .all(|c| classes.contains(&c.as_str()))
        {
            return false;
        }
        compound.attrs.iter().all(|(key, want)| {
            attrs.iter().find(|(k, _)| k == key).is_some_and(|(_, v)| {
                if key == "type" {
                    type_value_matches(v, want)
                } else {
                    v == want
                }
            })
        })
    }

    /// Every node below `id` in document order. Iterative, because scraped
    /// pages choose the nesting depth.
    fn descendants(&self, id: NodeId, out: &mut Vec<NodeId>) {
        let mut pending: Vec<NodeId> = self.nodes[id].children.iter().rev().copied().collect();
        while let Some(node) = pending.pop() {
            out.push(node);
            pending.extend(self.nodes[node].children.iter().rev());
        }
    }

    /// `scope.select(selector)`.
    pub fn select(&self, scope: NodeId, selector: &str) -> Vec<NodeId> {
        let compounds = parse_selector(selector);
        let mut all = Vec::new();
        self.descendants(scope, &mut all);
        all.into_iter()
            .filter(|&id| compounds.iter().any(|c| self.matches(id, c)))
            .collect()
    }

    pub fn select_one(&self, scope: NodeId, selector: &str) -> Option<NodeId> {
        self.select(scope, selector).into_iter().next()
    }

    /// `tag.get_text(separator, strip=strip)`.
    pub fn get_text(&self, id: NodeId, separator: &str, strip: bool) -> String {
        let wanted: &[StrKind] = match self
            .element(id)
            .and_then(|(name, _)| string_container(name))
        {
            Some(StrKind::Script) => &[StrKind::Script],
            Some(StrKind::Stylesheet) => &[StrKind::Stylesheet],
            Some(StrKind::Template) => &[StrKind::Template],
            Some(StrKind::RubyText) => &[StrKind::RubyText],
            Some(StrKind::RubyParen) => &[StrKind::RubyParen],
            _ => &[StrKind::Navigable, StrKind::CData],
        };
        let mut all = Vec::new();
        self.descendants(id, &mut all);
        let mut parts: Vec<&str> = Vec::new();
        for node in all {
            if let Kind::Text { text, kind } = &self.nodes[node].kind
                && wanted.contains(kind)
            {
                let piece = if strip {
                    pystr::strip(text)
                } else {
                    text.as_str()
                };
                if strip && piece.is_empty() {
                    continue;
                }
                parts.push(piece);
            }
        }
        parts.join(separator)
    }
}

fn is_multi_valued(tag: &str, key: &str) -> bool {
    matches!(key, "class" | "accesskey" | "dropzone")
        || matches!(
            (tag, key),
            ("a" | "link", "rel" | "rev")
                | ("td" | "th", "headers")
                | ("form", "accept-charset")
                | ("object", "archive")
                | ("area", "rel")
                | ("icon", "sizes")
                | ("iframe", "sandbox")
                | ("output", "for")
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(html: &str) -> String {
        let doc = parse(html);
        doc.get_text(doc.root(), "", false)
    }

    #[test]
    fn quirks_match_bs4() {
        assert_eq!(text("<p>a&#12b\nc d<b>x</b>"), "a\u{c}b");
        assert_eq!(text("<p>&#xAZ q"), "\nZ q");
        assert_eq!(
            text("<p>x &amp. y &amp;z &copy &notarealentity; &#150; &#0; &#x110000;"),
            "x &amp. y &z \u{a9} &notarealentity \u{2013} \u{fffd} \u{fffd}"
        );
        assert_eq!(
            text("<div>a<script>b</script><template>c<i>d</i></template><rt>e</rt>f</div>"),
            "af"
        );
        assert_eq!(text("<img src=x>after</img>tail<br/>end"), "aftertailend");
        assert_eq!(text("<p>  \n  </p><pre>  </pre>"), "\n  ");
        assert_eq!(
            text("<!-- c --><!DOCTYPE html><![CDATA[cd]]><?pi x?>text"),
            "cdtext"
        );
        assert_eq!(text("<a href=\"/albums/1\" title=\"t <b>\">x</a"), "x");
        assert_eq!(text("<p>&#x41;&#65;&#65x&lt;&ltx</p>"), "AAAx<&ltx");
    }

    #[test]
    fn attributes_match_html_parser() {
        let doc = parse("<p a=\"1\" a=\"2\" b c='x' d= e=\"&amp;&lt=&#65;\">t</p>");
        let p = doc.select_one(doc.root(), "p").unwrap();
        assert_eq!(doc.attr(p, "a"), Some("2"));
        assert_eq!(doc.attr(p, "b"), Some(""));
        assert_eq!(doc.attr(p, "c"), Some("x"));
        assert_eq!(doc.attr(p, "d"), Some("e=\"&&lt=A\""));
    }

    #[test]
    fn type_attribute_is_case_insensitive() {
        let doc = parse(
            "<script type=\"application/ld+json\n\">1</script><script type=\"APPLICATION/LD+JSON\">2</script>\
             <script type=\"appl\u{131}cation/ld+json\">3</script><script type=\"x\">4</script>",
        );
        let found: Vec<String> = doc
            .select(doc.root(), "script[type=\"application/ld+json\"]")
            .into_iter()
            .map(|id| doc.get_text(id, "", false))
            .collect();
        assert_eq!(found, ["1", "2", "3"]);
    }
}
