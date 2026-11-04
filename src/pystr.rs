//! Python `str` semantics the port depends on: character classes, strip,
//! `repr`, `splitlines`, and the `int()`/`float()` text parsers.

use crate::pytables;

fn in_ranges(table: &[(u32, u32)], c: char) -> bool {
    let cp = c as u32;
    table
        .binary_search_by(|&(lo, hi)| {
            if hi < cp {
                std::cmp::Ordering::Less
            } else if lo > cp {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok()
}

/// `str.isspace()`, which is also what `\s` matches in a `str` pattern.
pub fn is_space(c: char) -> bool {
    in_ranges(pytables::SPACE, c)
}

/// `str.isalnum()`.
pub fn is_alnum(c: char) -> bool {
    in_ranges(pytables::ALNUM, c)
}

/// What `\w` matches in a `str` pattern.
pub fn is_word(c: char) -> bool {
    c == '_' || is_alnum(c)
}

/// `str.isdigit()`.
pub fn is_digit(c: char) -> bool {
    in_ranges(pytables::DIGIT, c)
}

/// `str.isprintable()`.
pub fn is_printable(c: char) -> bool {
    in_ranges(pytables::PRINTABLE, c)
}

/// `unicodedata.category(c)[0] in "LN"`.
pub fn is_letter_or_number(c: char) -> bool {
    in_ranges(pytables::LETTER_OR_NUMBER, c)
}

/// `unicodedata.decimal(c)`.
pub fn decimal_value(c: char) -> Option<u32> {
    let cp = c as u32;
    let table = pytables::DECIMAL;
    let index = table.partition_point(|&(_, hi, _)| hi < cp);
    let &(lo, hi, base) = table.get(index)?;
    (lo <= cp && cp <= hi).then(|| base + (cp - lo))
}

pub fn strip(s: &str) -> &str {
    s.trim_matches(is_space)
}

pub fn lstrip(s: &str) -> &str {
    s.trim_start_matches(is_space)
}

pub fn rstrip(s: &str) -> &str {
    s.trim_end_matches(is_space)
}

/// `str.isdigit()` over a whole string (false when empty).
pub fn all_digits(s: &str) -> bool {
    !s.is_empty() && s.chars().all(is_digit)
}

/// The first `n` characters, as Python's `s[:n]` counts them.
pub fn prefix_chars(s: &str, n: usize) -> &str {
    match s.char_indices().nth(n) {
        Some((index, _)) => &s[..index],
        None => s,
    }
}

pub fn char_len(s: &str) -> usize {
    s.chars().count()
}

/// `str.splitlines()`.
pub fn splitlines(s: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut chars = s.char_indices().peekable();
    while let Some((index, c)) = chars.next() {
        let is_break = matches!(
            c,
            '\n' | '\r'
                | '\x0b'
                | '\x0c'
                | '\x1c'
                | '\x1d'
                | '\x1e'
                | '\u{85}'
                | '\u{2028}'
                | '\u{2029}'
        );
        if !is_break {
            continue;
        }
        lines.push(&s[start..index]);
        let mut end = index + c.len_utf8();
        if c == '\r'
            && let Some(&(next, '\n')) = chars.peek()
        {
            chars.next();
            end = next + 1;
        }
        start = end;
    }
    if start < s.len() {
        lines.push(&s[start..]);
    }
    lines
}

/// `repr(s)` for a `str`.
pub fn repr(s: &str) -> String {
    let quote = if s.contains('\'') && !s.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::with_capacity(s.len() + 2);
    out.push(quote);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if (c as u32) < 0x7f && (c as u32) >= 0x20 => out.push(c),
            c if is_printable(c) => out.push(c),
            c => {
                let cp = c as u32;
                if cp <= 0xff {
                    out.push_str(&format!("\\x{cp:02x}"));
                } else if cp <= 0xffff {
                    out.push_str(&format!("\\u{cp:04x}"));
                } else {
                    out.push_str(&format!("\\U{cp:08x}"));
                }
            }
        }
    }
    out.push(quote);
    out
}

/// Replace Unicode decimal digits with ASCII and Unicode spaces with ' ', as
/// `int()` and `float()` do before parsing; `None` if a character is neither.
fn transform_decimal_and_space(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii() {
                c
            } else if is_space(c) {
                ' '
            } else if let Some(d) = decimal_value(c) {
                char::from_digit(d, 10).unwrap_or(c)
            } else {
                c
            }
        })
        .collect()
}

/// Digits with PEP 515 underscores, each `_` between two digits.
fn strip_digit_underscores(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'_' {
            let before = i > 0 && bytes[i - 1].is_ascii_digit();
            let after = bytes.get(i + 1).is_some_and(u8::is_ascii_digit);
            if !(before && after) {
                return None;
            }
        } else {
            out.push(b as char);
        }
    }
    Some(out)
}

/// The value `int(text)` gives, saturated to the i128 range; `None` where
/// `int()` raises ValueError (including Python's 4300-digit limit).
pub fn parse_int(text: &str) -> Option<i128> {
    let text = transform_decimal_and_space(text);
    let body = text.trim_matches(' ');
    let (negative, digits) = match body.as_bytes().first() {
        Some(b'-') => (true, &body[1..]),
        Some(b'+') => (false, &body[1..]),
        _ => (false, body),
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit() || b == b'_') {
        return None;
    }
    let digits = strip_digit_underscores(digits)?;
    if digits.trim_start_matches('0').len() > 4300 {
        return None;
    }
    let mut value: i128 = 0;
    for b in digits.bytes() {
        value = value
            .saturating_mul(10)
            .saturating_add(i128::from(b - b'0'));
    }
    Some(if negative { -value } else { value })
}

/// `float(text)`; `None` where Python raises ValueError.
pub fn parse_float(text: &str) -> Option<f64> {
    let text = transform_decimal_and_space(text);
    let body = text.trim_matches(' ');
    let unsigned = body.strip_prefix(['+', '-']).unwrap_or(body);
    let lower = unsigned.to_ascii_lowercase();
    if matches!(lower.as_str(), "inf" | "infinity" | "nan") {
        let value = if lower == "nan" {
            f64::NAN
        } else {
            f64::INFINITY
        };
        return Some(if body.starts_with('-') { -value } else { value });
    }
    let valid = {
        let b = unsigned.as_bytes();
        let mut i = 0;
        let int_start = i;
        while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'_') {
            i += 1;
        }
        let int_digits = i - int_start;
        let mut frac_digits = 0;
        if i < b.len() && b[i] == b'.' {
            i += 1;
            let start = i;
            while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'_') {
                i += 1;
            }
            frac_digits = i - start;
        }
        let mut ok = int_digits + frac_digits > 0;
        if ok && i < b.len() && (b[i] == b'e' || b[i] == b'E') {
            i += 1;
            if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
                i += 1;
            }
            let start = i;
            while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'_') {
                i += 1;
            }
            ok = i > start;
        }
        ok && i == b.len()
    };
    if !valid {
        return None;
    }
    // Underscores must sit between digits within each digit run.
    let cleaned = strip_digit_underscores(body)?;
    cleaned.parse::<f64>().ok()
}

/// `format!("{value:e}")`, but where two spellings are equally short, the one
/// CPython's `repr` prints: the nearer to `value`, an exact tie going to the even
/// digit. Rust's shortest formatting can break that tie upwards: `1 + 2**-17` is
/// `1.0000076293945312` in Python, not `…313`. `{:.Ne}` rounds exactly with ties
/// to even, so its spelling is used whenever it still reads back as `value`.
fn shortest_exp(value: f64) -> String {
    let shortest = format!("{value:e}");
    let precision = shortest
        .bytes()
        .take_while(|&b| b != b'e')
        .filter(u8::is_ascii_digit)
        .count()
        - 1;
    let nearest = format!("{value:.precision$e}");
    if nearest.parse::<f64>() == Ok(value) {
        nearest
    } else {
        shortest
    }
}

/// `repr(float)`.
pub fn repr_float(value: f64) -> String {
    if value.is_nan() {
        return "nan".into();
    }
    if value.is_infinite() {
        return if value > 0.0 {
            "inf".into()
        } else {
            "-inf".into()
        };
    }
    if value == 0.0 {
        return if value.is_sign_negative() {
            "-0.0".into()
        } else {
            "0.0".into()
        };
    }
    let formatted = shortest_exp(value.abs());
    let (mantissa, exponent) = formatted.split_once('e').unwrap_or((&formatted, "0"));
    let exponent: i32 = exponent.parse().unwrap_or(0);
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let sign = if value < 0.0 { "-" } else { "" };
    if (-4..16).contains(&exponent) {
        let point = exponent + 1;
        let body = if point <= 0 {
            format!("0.{}{}", "0".repeat((-point) as usize), digits)
        } else if point as usize >= digits.len() {
            format!("{}{}.0", digits, "0".repeat(point as usize - digits.len()))
        } else {
            format!(
                "{}.{}",
                &digits[..point as usize],
                &digits[point as usize..]
            )
        };
        format!("{sign}{body}")
    } else {
        let mut body = digits[..1].to_string();
        if digits.len() > 1 {
            body.push('.');
            body.push_str(&digits[1..]);
        }
        let esign = if exponent < 0 { '-' } else { '+' };
        format!("{sign}{body}e{esign}{:02}", exponent.abs())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn float_repr_matches_python() {
        for (value, text) in [
            (1e16, "1e+16"),
            (1e15, "1000000000000000.0"),
            (0.0001, "0.0001"),
            (0.00001, "1e-05"),
            (-0.0, "-0.0"),
            (1.5e300, "1.5e+300"),
            (123456789012345678.0, "1.2345678901234568e+17"),
            (5e-324, "5e-324"),
            (0.1 + 0.2, "0.30000000000000004"),
            (100.0, "100.0"),
            (12.5, "12.5"),
        ] {
            assert_eq!(repr_float(value), text);
        }
    }

    #[test]
    fn int_and_float_parsing() {
        assert_eq!(parse_int(" 12 "), Some(12));
        assert_eq!(parse_int("1_000"), Some(1000));
        assert_eq!(parse_int("1__0"), None);
        assert_eq!(parse_int("\u{0661}\u{0662}"), Some(12));
        assert_eq!(parse_int("1.0"), None);
        assert_eq!(parse_float("1e3"), Some(1000.0));
        assert!(parse_float("nan").unwrap().is_nan());
        assert_eq!(parse_float("-Infinity"), Some(f64::NEG_INFINITY));
        assert_eq!(parse_float("1_0.5"), Some(10.5));
        assert_eq!(parse_float("1_.5"), None);
        assert_eq!(parse_float("."), None);
    }

    #[test]
    fn repr_quotes_like_python() {
        assert_eq!(repr("a'b"), "\"a'b\"");
        assert_eq!(repr("a\\b\n"), "'a\\\\b\\n'");
        assert_eq!(repr("\u{7f}\u{200b}é"), "'\\x7f\\u200bé'");
    }

    #[test]
    fn float_repr_matches_cpython_corpus() {
        // CPython 3.15's repr for doubles chosen to exercise ties between two
        // equally short spellings: bits in hex, then the repr.
        assert_eq!(repr_float(1.0 + 2f64.powi(-17)), "1.0000076293945312");
        let wrong: Vec<String> = include_str!("../tests/fixtures/float_repr.txt")
            .lines()
            .filter_map(|line| {
                let (bits, expected) = line.split_once(' ').expect("bits and repr");
                let actual = repr_float(f64::from_bits(u64::from_str_radix(bits, 16).unwrap()));
                (actual != expected).then(|| format!("{bits}: python {expected}, rust {actual}"))
            })
            .collect();
        assert!(
            wrong.is_empty(),
            "{} wrong: {:?}",
            wrong.len(),
            &wrong[..wrong.len().min(10)]
        );
    }
}
