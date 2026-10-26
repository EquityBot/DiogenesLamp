//! JSON with Python's semantics: `json.loads` (the C scanner, including its
//! error messages), `json.dumps(..., ensure_ascii=False)`, dict ordering,
//! arbitrary-size integers and Python's cross-type equality.

use indexmap::IndexMap;
use std::fmt::Write as _;

use crate::pystr;

pub type Map = IndexMap<String, Value>;

#[derive(Clone, Debug)]
pub enum Value {
    Null,
    Bool(bool),
    /// An integer that fits i128.
    Int(i128),
    /// A larger integer, as canonical decimal text.
    BigInt(String),
    Float(f64),
    Str(String),
    Array(Vec<Value>),
    Object(Map),
}

impl Value {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&Vec<Value>> {
        match self {
            Value::Array(items) => Some(items),
            _ => None,
        }
    }

    pub fn as_array_mut(&mut self) -> Option<&mut Vec<Value>> {
        match self {
            Value::Array(items) => Some(items),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&Map> {
        match self {
            Value::Object(map) => Some(map),
            _ => None,
        }
    }

    pub fn as_object_mut(&mut self) -> Option<&mut Map> {
        match self {
            Value::Object(map) => Some(map),
            _ => None,
        }
    }

    /// `type(value) is int` (bool excluded).
    pub fn is_int(&self) -> bool {
        matches!(self, Value::Int(_) | Value::BigInt(_))
    }

    /// Python truthiness.
    pub fn truthy(&self) -> bool {
        match self {
            Value::Null => false,
            Value::Bool(b) => *b,
            Value::Int(i) => *i != 0,
            Value::BigInt(_) => true,
            Value::Float(f) => *f != 0.0,
            Value::Str(s) => !s.is_empty(),
            Value::Array(items) => !items.is_empty(),
            Value::Object(map) => !map.is_empty(),
        }
    }

    /// `str(value)` for the scalar types the port inspects that way.
    pub fn py_str(&self) -> String {
        match self {
            Value::Null => "None".into(),
            Value::Bool(true) => "True".into(),
            Value::Bool(false) => "False".into(),
            Value::Int(i) => i.to_string(),
            Value::BigInt(s) => s.clone(),
            Value::Float(f) => pystr::repr_float(*f),
            Value::Str(s) => s.clone(),
            other => dumps(other, Style::Default),
        }
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::Str(s.to_string())
    }
}

impl From<String> for Value {
    fn from(s: String) -> Self {
        Value::Str(s)
    }
}

impl From<i64> for Value {
    fn from(i: i64) -> Self {
        Value::Int(i128::from(i))
    }
}

impl From<usize> for Value {
    fn from(i: usize) -> Self {
        Value::Int(i as i128)
    }
}

impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Value::Bool(b)
    }
}

impl From<Map> for Value {
    fn from(map: Map) -> Self {
        Value::Object(map)
    }
}

impl From<Vec<Value>> for Value {
    fn from(items: Vec<Value>) -> Self {
        Value::Array(items)
    }
}

/// Exact numeric value of an int or bool, as a sign and decimal magnitude.
fn exact_int(value: &Value) -> Option<(bool, String)> {
    match value {
        Value::Bool(b) => Some((false, if *b { "1" } else { "0" }.into())),
        Value::Int(i) => Some((*i < 0, i.unsigned_abs().to_string())),
        Value::BigInt(s) => Some(match s.strip_prefix('-') {
            Some(rest) => (true, rest.to_string()),
            None => (false, s.clone()),
        }),
        _ => None,
    }
}

fn float_equals_int(f: f64, int: &(bool, String)) -> bool {
    if !f.is_finite() || f.fract() != 0.0 {
        return false;
    }
    let text = format!("{:.0}", f.abs());
    let negative = f < 0.0 && text != "0";
    let (int_negative, digits) = int;
    let int_negative = *int_negative && digits != "0";
    negative == int_negative && text == *digits
}

/// Python `==` between two JSON-derived values.
pub fn py_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Null, Value::Null) => true,
        (Value::Str(x), Value::Str(y)) => x == y,
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| py_eq(p, q))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| py_eq(v, w)))
        }
        (Value::Float(x), Value::Float(y)) => x == y,
        (Value::Float(f), other) | (other, Value::Float(f)) => {
            exact_int(other).is_some_and(|int| float_equals_int(*f, &int))
        }
        _ => match (exact_int(a), exact_int(b)) {
            (Some((sa, da)), Some((sb, db))) => {
                let za = da == "0";
                (za && db == "0") || (sa == sb && da == db)
            }
            _ => false,
        },
    }
}

/// A `json.loads` failure: `JSONDecodeError`, or the `ValueError` Python
/// raises for an integer beyond its digit limit.
#[derive(Debug, Clone)]
pub struct DecodeError(pub String);

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

struct Parser<'a> {
    s: &'a str,
    b: &'a [u8],
    depth: usize,
}

enum Fail {
    /// StopIteration(idx): no value starts here.
    Stop(usize),
    Error(DecodeError),
}

const MAX_DEPTH: usize = 4000;

impl<'a> Parser<'a> {
    fn error(&self, msg: &str, pos: usize) -> Fail {
        Fail::Error(decode_error(msg, self.s, pos))
    }

    fn skip_ws(&self, mut i: usize) -> usize {
        while i < self.b.len() && matches!(self.b[i], b' ' | b'\t' | b'\n' | b'\r') {
            i += 1;
        }
        i
    }

    fn scan_once(&mut self, idx: usize) -> Result<(Value, usize), Fail> {
        let b = self.b;
        let Some(&c) = b.get(idx) else {
            return Err(Fail::Stop(idx));
        };
        match c {
            b'"' => {
                let (s, end) = self.scan_string(idx + 1)?;
                Ok((Value::Str(s), end))
            }
            b'{' => self.nested(|p| p.parse_object(idx + 1)),
            b'[' => self.nested(|p| p.parse_array(idx + 1)),
            b'n' if b[idx..].starts_with(b"null") => Ok((Value::Null, idx + 4)),
            b't' if b[idx..].starts_with(b"true") => Ok((Value::Bool(true), idx + 4)),
            b'f' if b[idx..].starts_with(b"false") => Ok((Value::Bool(false), idx + 5)),
            b'N' if b[idx..].starts_with(b"NaN") => Ok((Value::Float(f64::NAN), idx + 3)),
            b'I' if b[idx..].starts_with(b"Infinity") => Ok((Value::Float(f64::INFINITY), idx + 8)),
            b'-' if b[idx..].starts_with(b"-Infinity") => {
                Ok((Value::Float(f64::NEG_INFINITY), idx + 9))
            }
            _ => self.match_number(idx),
        }
    }

    fn nested(
        &mut self,
        f: impl FnOnce(&mut Self) -> Result<(Value, usize), Fail>,
    ) -> Result<(Value, usize), Fail> {
        // shortcut: Python raises RecursionError near its C stack limit; a
        // fixed nesting bound keeps the native stack safe instead.
        // Switch to an iterative parser if a real document reaches it.
        if self.depth >= MAX_DEPTH {
            return Err(Fail::Error(DecodeError(
                "maximum recursion depth exceeded while decoding a JSON document".into(),
            )));
        }
        self.depth += 1;
        let result = f(self);
        self.depth -= 1;
        result
    }

    fn match_number(&self, start: usize) -> Result<(Value, usize), Fail> {
        let b = self.b;
        let end_idx = b.len().wrapping_sub(1);
        let mut idx = start;
        if b[idx] == b'-' {
            idx += 1;
            if idx > end_idx || b.is_empty() {
                return Err(Fail::Stop(start));
            }
        }
        match b[idx] {
            b'1'..=b'9' => {
                idx += 1;
                while idx <= end_idx && b[idx].is_ascii_digit() {
                    idx += 1;
                }
            }
            b'0' => idx += 1,
            _ => return Err(Fail::Stop(start)),
        }
        let mut is_float = false;
        if idx < end_idx && b[idx] == b'.' && b[idx + 1].is_ascii_digit() {
            is_float = true;
            idx += 2;
            while idx <= end_idx && b[idx].is_ascii_digit() {
                idx += 1;
            }
        }
        if idx < end_idx && (b[idx] == b'e' || b[idx] == b'E') {
            let e_start = idx;
            idx += 1;
            if idx < end_idx && (b[idx] == b'-' || b[idx] == b'+') {
                idx += 1;
            }
            while idx <= end_idx && b[idx].is_ascii_digit() {
                idx += 1;
            }
            if b[idx - 1].is_ascii_digit() {
                is_float = true;
            } else {
                idx = e_start;
            }
        }
        let text = &self.s[start..idx];
        if is_float {
            let value: f64 = text.parse().unwrap_or(f64::NAN);
            return Ok((Value::Float(value), idx));
        }
        Ok((parse_int_text(text).map_err(Fail::Error)?, idx))
    }

    fn scan_string(&self, mut end: usize) -> Result<(String, usize), Fail> {
        let b = self.b;
        let begin = end - 1;
        let mut out = String::new();
        loop {
            let chunk_start = end;
            while end < b.len() && b[end] != b'"' && b[end] != b'\\' && b[end] >= 0x20 {
                end += 1;
            }
            out.push_str(&self.s[chunk_start..end]);
            if end >= b.len() {
                return Err(self.error("Unterminated string starting at", begin));
            }
            match b[end] {
                b'"' => return Ok((out, end + 1)),
                b'\\' => {}
                _ => return Err(self.error("Invalid control character at", end)),
            }
            let backslash = end;
            end += 1;
            if end >= b.len() {
                return Err(self.error("Unterminated string starting at", begin));
            }
            let esc = b[end];
            if esc != b'u' {
                let c = match esc {
                    b'"' => '"',
                    b'\\' => '\\',
                    b'/' => '/',
                    b'b' => '\u{8}',
                    b'f' => '\u{c}',
                    b'n' => '\n',
                    b'r' => '\r',
                    b't' => '\t',
                    _ => return Err(self.error("Invalid \\escape", backslash)),
                };
                out.push(c);
                end += 1;
                continue;
            }
            // \uXXXX, with the C scanner's bounds and positions.
            let mut next = end + 1;
            let mut stop = next + 4;
            if stop >= b.len() {
                return Err(self.error("Invalid \\uXXXX escape", next - 1));
            }
            let mut c = 0u32;
            while next < stop {
                let Some(d) = (b[next] as char).to_digit(16) else {
                    return Err(self.error("Invalid \\uXXXX escape", stop - 5));
                };
                c = (c << 4) | d;
                next += 1;
            }
            if (0xd800..=0xdbff).contains(&c) && stop + 6 < b.len() {
                let first = b[next];
                next += 1;
                let second = b[next];
                next += 1;
                if first == b'\\' && second == b'u' {
                    stop += 6;
                    let mut c2 = 0u32;
                    while next < stop {
                        let Some(d) = (b[next] as char).to_digit(16) else {
                            return Err(self.error("Invalid \\uXXXX escape", stop - 5));
                        };
                        c2 = (c2 << 4) | d;
                        next += 1;
                    }
                    if (0xdc00..=0xdfff).contains(&c2) {
                        c = 0x10000 + (((c - 0xd800) << 10) | (c2 - 0xdc00));
                    } else {
                        stop -= 6;
                    }
                }
            }
            end = stop;
            match char::from_u32(c) {
                Some(ch) => out.push(ch),
                None => {
                    // shortcut: Python keeps a lone surrogate in the str; a
                    // Rust String cannot hold one, so such a document is refused.
                    // Use a surrogate-preserving string type if a real input carries one.
                    return Err(Fail::Error(DecodeError(format!(
                        "lone surrogate \\u{c:04x} is not supported: line {} column {} (char {})",
                        line_col(self.s, backslash).0,
                        line_col(self.s, backslash).1,
                        char_index(self.s, backslash)
                    ))));
                }
            }
        }
    }

    fn parse_object(&mut self, start: usize) -> Result<(Value, usize), Fail> {
        let b = self.b;
        let ws = |c: Option<&u8>| matches!(c, Some(b' ' | b'\t' | b'\n' | b'\r'));
        let mut end = start;
        let mut map = Map::new();
        let mut nextchar = b.get(end);
        if nextchar != Some(&b'"') {
            if ws(nextchar) || nextchar.is_none() {
                end = self.skip_ws(end);
                nextchar = b.get(end);
            }
            if nextchar == Some(&b'}') {
                return Ok((Value::Object(map), end + 1));
            } else if nextchar != Some(&b'"') {
                return Err(self.error("Expecting property name enclosed in double quotes", end));
            }
        }
        end += 1;
        loop {
            let (key, after_key) = self.scan_string(end)?;
            end = after_key;
            if b.get(end) != Some(&b':') {
                end = self.skip_ws(end);
                if b.get(end) != Some(&b':') {
                    return Err(self.error("Expecting ':' delimiter", end));
                }
            }
            end += 1;
            end = self.skip_ws(end);
            let (value, after_value) = match self.scan_once(end) {
                Ok(found) => found,
                Err(Fail::Stop(at)) => return Err(self.error("Expecting value", at)),
                Err(error) => return Err(error),
            };
            end = after_value;
            map.insert(key, value);
            end = self.skip_ws(end);
            let nextchar = b.get(end).copied();
            end += 1;
            match nextchar {
                Some(b'}') => break,
                Some(b',') => {}
                _ => return Err(self.error("Expecting ',' delimiter", end - 1)),
            }
            let comma = end - 1;
            end = self.skip_ws(end);
            let nextchar = b.get(end).copied();
            end += 1;
            if nextchar != Some(b'"') {
                if nextchar == Some(b'}') {
                    return Err(self.error("Illegal trailing comma before end of object", comma));
                }
                return Err(
                    self.error("Expecting property name enclosed in double quotes", end - 1)
                );
            }
        }
        Ok((Value::Object(map), end))
    }

    fn parse_array(&mut self, start: usize) -> Result<(Value, usize), Fail> {
        let b = self.b;
        let mut end = self.skip_ws(start);
        let mut items = Vec::new();
        if b.get(end) == Some(&b']') {
            return Ok((Value::Array(items), end + 1));
        }
        loop {
            let (value, after) = match self.scan_once(end) {
                Ok(found) => found,
                Err(Fail::Stop(at)) => return Err(self.error("Expecting value", at)),
                Err(error) => return Err(error),
            };
            items.push(value);
            end = self.skip_ws(after);
            let nextchar = b.get(end).copied();
            end += 1;
            match nextchar {
                Some(b']') => break,
                Some(b',') => {}
                _ => return Err(self.error("Expecting ',' delimiter", end - 1)),
            }
            let comma = end - 1;
            end = self.skip_ws(end);
            if b.get(end) == Some(&b']') {
                return Err(self.error("Illegal trailing comma before end of array", comma));
            }
        }
        Ok((Value::Array(items), end))
    }
}

fn parse_int_text(text: &str) -> Result<Value, DecodeError> {
    let (negative, digits) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    if digits.len() > 4300 {
        return Err(DecodeError(format!(
            "Exceeds the limit (4300 digits) for integer string conversion: value has {} \
             digits; use sys.set_int_max_str_digits() to increase the limit",
            digits.len()
        )));
    }
    if let Ok(value) = text.parse::<i128>() {
        return Ok(Value::Int(value));
    }
    let digits = digits.trim_start_matches('0');
    Ok(Value::BigInt(if negative {
        format!("-{digits}")
    } else {
        digits.to_string()
    }))
}

fn char_index(s: &str, byte: usize) -> usize {
    s[..byte.min(s.len())].chars().count()
}

fn line_col(s: &str, byte: usize) -> (usize, usize) {
    let before = &s[..byte.min(s.len())];
    let line = before.matches('\n').count() + 1;
    let col = match before.rfind('\n') {
        Some(nl) => before[nl + 1..].chars().count() + 1,
        None => before.chars().count() + 1,
    };
    (line, col)
}

fn decode_error(msg: &str, s: &str, byte: usize) -> DecodeError {
    let (line, col) = line_col(s, byte);
    DecodeError(format!(
        "{msg}: line {line} column {col} (char {})",
        char_index(s, byte)
    ))
}

/// `json.loads(text)` for a `str`.
pub fn loads(text: &str) -> Result<Value, DecodeError> {
    if text.starts_with('\u{feff}') {
        return Err(decode_error(
            "Unexpected UTF-8 BOM (decode using utf-8-sig)",
            text,
            0,
        ));
    }
    let mut parser = Parser {
        s: text,
        b: text.as_bytes(),
        depth: 0,
    };
    let start = parser.skip_ws(0);
    let (value, end) = match parser.scan_once(start) {
        Ok(found) => found,
        Err(Fail::Stop(at)) => return Err(decode_error("Expecting value", text, at)),
        Err(Fail::Error(error)) => return Err(error),
    };
    let end = parser.skip_ws(end);
    if end != text.len() {
        return Err(decode_error("Extra data", text, end));
    }
    Ok(value)
}

/// `json.loads(data)` for `bytes`: encoding detection, then decode with
/// `surrogatepass`.
pub fn loads_bytes(data: &[u8]) -> Result<Value, String> {
    let text = crate::pyerr::decode_json_bytes(data)?;
    loads(&text).map_err(|e| e.0)
}

#[derive(Clone, Copy, PartialEq)]
pub enum Style {
    /// `separators=(",", ":")`.
    Compact,
    /// The default `(", ", ": ")`.
    Default,
    /// `indent=n`.
    Indent(usize),
}

/// `json.dumps(value, ensure_ascii=False, ...)`.
pub fn dumps(value: &Value, style: Style) -> String {
    let mut out = String::new();
    write_value(&mut out, value, style, 0);
    out
}

pub fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    write_str(&mut out, s);
    out
}

fn write_str(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn float_text(f: f64) -> String {
    if f.is_nan() {
        "NaN".into()
    } else if f.is_infinite() {
        if f > 0.0 {
            "Infinity".into()
        } else {
            "-Infinity".into()
        }
    } else {
        pystr::repr_float(f)
    }
}

fn write_value(out: &mut String, value: &Value, style: Style, level: usize) {
    let (item_sep, key_sep) = match style {
        Style::Compact => (",", ":"),
        Style::Default => (", ", ": "),
        Style::Indent(_) => (",", ": "),
    };
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Int(i) => {
            let _ = write!(out, "{i}");
        }
        Value::BigInt(s) => out.push_str(s),
        Value::Float(f) => out.push_str(&float_text(*f)),
        Value::Str(s) => write_str(out, s),
        Value::Array(items) => {
            if items.is_empty() {
                out.push_str("[]");
                return;
            }
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(item_sep);
                }
                newline_indent(out, style, level + 1);
                write_value(out, item, style, level + 1);
            }
            newline_indent(out, style, level);
            out.push(']');
        }
        Value::Object(map) => {
            if map.is_empty() {
                out.push_str("{}");
                return;
            }
            out.push('{');
            for (i, (key, item)) in map.iter().enumerate() {
                if i > 0 {
                    out.push_str(item_sep);
                }
                newline_indent(out, style, level + 1);
                write_str(out, key);
                out.push_str(key_sep);
                write_value(out, item, style, level + 1);
            }
            newline_indent(out, style, level);
            out.push('}');
        }
    }
}

fn newline_indent(out: &mut String, style: Style, level: usize) {
    if let Style::Indent(n) = style {
        out.push('\n');
        out.push_str(&" ".repeat(n * level));
    }
}

/// Build an object from key/value pairs, Python-dict style.
#[macro_export]
macro_rules! obj {
    ($($key:expr => $value:expr),* $(,)?) => {{
        let mut map = $crate::pyjson::Map::new();
        $(map.insert(($key).to_string(), $crate::pyjson::Value::from($value));)*
        map
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    fn err(text: &str) -> String {
        loads(text).unwrap_err().0
    }

    #[test]
    fn errors_match_the_c_scanner() {
        let cases = [
            (
                "\"a\u{1}\"",
                "Invalid control character at: line 1 column 3 (char 2)",
            ),
            (
                "{\"a\" 1}",
                "Expecting ':' delimiter: line 1 column 6 (char 5)",
            ),
            (
                "{\"a\":1,}",
                "Illegal trailing comma before end of object: line 1 column 7 (char 6)",
            ),
            (
                "[1,]",
                "Illegal trailing comma before end of array: line 1 column 3 (char 2)",
            ),
            ("[1 2]", "Expecting ',' delimiter: line 1 column 4 (char 3)"),
            (
                "{1:2}",
                "Expecting property name enclosed in double quotes: line 1 column 2 (char 1)",
            ),
            ("\"\\x\"", "Invalid \\escape: line 1 column 2 (char 1)"),
            (
                "\"\\u12\"",
                "Invalid \\uXXXX escape: line 1 column 3 (char 2)",
            ),
            ("", "Expecting value: line 1 column 1 (char 0)"),
            ("  ", "Expecting value: line 1 column 3 (char 2)"),
            ("[1] x", "Extra data: line 1 column 5 (char 4)"),
            ("01", "Extra data: line 1 column 2 (char 1)"),
            ("1.", "Extra data: line 1 column 2 (char 1)"),
            (
                "\"abc",
                "Unterminated string starting at: line 1 column 1 (char 0)",
            ),
            ("{\"a\":}", "Expecting value: line 1 column 6 (char 5)"),
            ("[", "Expecting value: line 1 column 2 (char 1)"),
            (
                "{",
                "Expecting property name enclosed in double quotes: line 1 column 2 (char 1)",
            ),
            (
                "{\"a\"",
                "Expecting ':' delimiter: line 1 column 5 (char 4)",
            ),
            (
                "{\"a\":1",
                "Expecting ',' delimiter: line 1 column 7 (char 6)",
            ),
            (
                "\u{feff}{}",
                "Unexpected UTF-8 BOM (decode using utf-8-sig): line 1 column 1 (char 0)",
            ),
            ("[1,\n2,\n x]", "Expecting value: line 3 column 2 (char 8)"),
            (
                "{\"a\":1,\"b\"}",
                "Expecting ':' delimiter: line 1 column 11 (char 10)",
            ),
            ("[,]", "Expecting value: line 1 column 2 (char 1)"),
            (
                "\"\\",
                "Unterminated string starting at: line 1 column 1 (char 0)",
            ),
            (
                "{\"a\" : 1 , }",
                "Illegal trailing comma before end of object: line 1 column 10 (char 9)",
            ),
            (
                "[1 ,]",
                "Illegal trailing comma before end of array: line 1 column 4 (char 3)",
            ),
            ("1e", "Extra data: line 1 column 2 (char 1)"),
            ("1.5e", "Extra data: line 1 column 4 (char 3)"),
            ("[1e]", "Expecting ',' delimiter: line 1 column 3 (char 2)"),
            ("[-NaN]", "Expecting value: line 1 column 2 (char 1)"),
            ("1\n\n2", "Extra data: line 3 column 1 (char 3)"),
        ];
        for (text, expected) in cases {
            assert_eq!(err(text), expected, "{text:?}");
        }
    }

    #[test]
    fn values_and_dumps_match_python() {
        let value = loads("{\"a\":1,\"a\":2,\"b\":3}").unwrap();
        assert_eq!(dumps(&value, Style::Default), "{\"a\": 2, \"b\": 3}");
        let value = loads("[-0, -0.0, 1E5, 1e-7, 1e400, 123456789012345678901234567890]").unwrap();
        assert_eq!(
            dumps(&value, Style::Compact),
            "[0,-0.0,100000.0,1e-07,Infinity,123456789012345678901234567890]"
        );
        let value =
            loads("{\"a\": [], \"b\": {}, \"c\": [1, {\"d\": \"\u{e9}\u{7f}\\u001f\u{2028}\"}]}")
                .unwrap();
        assert_eq!(
            dumps(&value, Style::Indent(2)),
            "{\n  \"a\": [],\n  \"b\": {},\n  \"c\": [\n    1,\n    {\n      \"d\": \"\u{e9}\u{7f}\\u001f\u{2028}\"\n    }\n  ]\n}"
        );
    }

    #[test]
    fn python_equality_crosses_numeric_types() {
        assert!(py_eq(&Value::Int(1), &Value::Float(1.0)));
        assert!(py_eq(&Value::Bool(true), &Value::Int(1)));
        assert!(!py_eq(&Value::Str("1".into()), &Value::Int(1)));
        assert!(py_eq(&Value::Float(-0.0), &Value::Int(0)));
    }
}
