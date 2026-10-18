//! The parts of Python's `ipaddress` the port uses: strict parsing, `str()`
//! and the special-purpose registries behind `is_global`.

use crate::pystr::repr;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ip {
    V4(u32),
    V6(u128),
}

fn parse_octet(octet: &str) -> Result<u32, String> {
    if octet.is_empty() {
        return Err("Empty octet not permitted".into());
    }
    if !octet.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!("Only decimal digits permitted in {}", repr(octet)));
    }
    if octet.len() > 3 {
        return Err(format!("At most 3 characters permitted in {}", repr(octet)));
    }
    if octet != "0" && octet.starts_with('0') {
        return Err(format!(
            "Leading zeros are not permitted in {}",
            repr(octet)
        ));
    }
    let value: u32 = octet.parse().unwrap_or(0);
    if value > 255 {
        return Err(format!("Octet {value} (> 255) not permitted"));
    }
    Ok(value)
}

/// `IPv4Address(text)`.
pub fn parse_v4(text: &str) -> Result<u32, String> {
    if text.is_empty() {
        return Err("Address cannot be empty".into());
    }
    let octets: Vec<&str> = text.split('.').collect();
    if octets.len() != 4 {
        return Err(format!("Expected 4 octets in {}", repr(text)));
    }
    let mut value = 0u32;
    for octet in octets {
        let part = parse_octet(octet).map_err(|e| format!("{e} in {}", repr(text)))?;
        value = (value << 8) | part;
    }
    Ok(value)
}

fn parse_hextet(hextet: &str) -> Result<u128, String> {
    if !hextet.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!("Only hex digits permitted in {}", repr(hextet)));
    }
    if hextet.len() > 4 {
        return Err(format!(
            "At most 4 characters permitted in {}",
            repr(hextet)
        ));
    }
    // int("", 16) raises: Python reports it through the same handler.
    u128::from_str_radix(hextet, 16)
        .map_err(|_| format!("invalid literal for int() with base 16: {}", repr(hextet)))
}

/// `IPv6Address(text)` without a scope id: the 128-bit value.
pub fn parse_v6_value(text: &str) -> Result<u128, String> {
    if text.is_empty() {
        return Err("Address cannot be empty".into());
    }
    if text.chars().count() > 45 {
        return Err(format!("At most 45 characters expected in {}", repr(text)));
    }
    let max_parts = 9;
    let mut parts: Vec<String> = text
        .splitn(max_parts + 1, ':')
        .map(str::to_string)
        .collect();
    if parts.len() < 3 {
        return Err(format!("At least 3 parts expected in {}", repr(text)));
    }
    if parts.last().is_some_and(|p| p.contains('.')) {
        let tail = parts.pop().unwrap_or_default();
        let v4 = parse_v4(&tail).map_err(|e| format!("{e} in {}", repr(text)))?;
        parts.push(format!("{:x}", (v4 >> 16) & 0xffff));
        parts.push(format!("{:x}", v4 & 0xffff));
    }
    if parts.len() > max_parts {
        return Err(format!("At most 8 colons permitted in {}", repr(text)));
    }
    let mut skip = None;
    for (i, part) in parts.iter().enumerate().take(parts.len() - 1).skip(1) {
        if part.is_empty() {
            if skip.is_some() {
                return Err(format!("At most one '::' permitted in {}", repr(text)));
            }
            skip = Some(i);
        }
    }
    let (hi, lo, skipped) = if let Some(index) = skip {
        let mut hi = index;
        let mut lo = parts.len() - index - 1;
        if parts[0].is_empty() {
            hi -= 1;
            if hi > 0 {
                return Err(format!(
                    "Leading ':' only permitted as part of '::' in {}",
                    repr(text)
                ));
            }
        }
        if parts[parts.len() - 1].is_empty() {
            lo -= 1;
            if lo > 0 {
                return Err(format!(
                    "Trailing ':' only permitted as part of '::' in {}",
                    repr(text)
                ));
            }
        }
        let skipped = 8i64 - (hi + lo) as i64;
        if skipped < 1 {
            return Err(format!(
                "Expected at most 7 other parts with '::' in {}",
                repr(text)
            ));
        }
        (hi, lo, skipped as usize)
    } else {
        if parts.len() != 8 {
            return Err(format!(
                "Exactly 8 parts expected without '::' in {}",
                repr(text)
            ));
        }
        if parts[0].is_empty() {
            return Err(format!(
                "Leading ':' only permitted as part of '::' in {}",
                repr(text)
            ));
        }
        if parts[7].is_empty() {
            return Err(format!(
                "Trailing ':' only permitted as part of '::' in {}",
                repr(text)
            ));
        }
        (8, 0, 0)
    };
    let wrap = |e: String| format!("{e} in {}", repr(text));
    let mut value: u128 = 0;
    for part in &parts[..hi] {
        value = (value << 16) | parse_hextet(part).map_err(wrap)?;
    }
    value = value.checked_shl(16 * skipped as u32).unwrap_or(0);
    for part in &parts[parts.len() - lo..] {
        value = (value << 16) | parse_hextet(part).map_err(wrap)?;
    }
    Ok(value)
}

/// An IPv6 address and its optional scope id.
pub fn parse_v6(text: &str) -> Result<(u128, Option<String>), String> {
    let (addr, scope) = match text.split_once('%') {
        None => (text, None),
        Some((addr, scope)) => {
            if scope.is_empty() || scope.contains('%') {
                return Err(format!("Invalid IPv6 address: \"{}\"", repr(text)));
            }
            (addr, Some(scope.to_string()))
        }
    };
    Ok((parse_v6_value(addr)?, scope))
}

/// `ipaddress.ip_address(text)`; a scope id is kept in the returned text.
pub fn ip_address(text: &str) -> Result<(Ip, Option<String>), String> {
    if let Ok(v4) = parse_v4(text) {
        return Ok((Ip::V4(v4), None));
    }
    if let Ok((v6, scope)) = parse_v6(text) {
        return Ok((Ip::V6(v6), scope));
    }
    Err(format!(
        "{} does not appear to be an IPv4 or IPv6 address",
        repr(text)
    ))
}

pub fn v4_str(value: u32) -> String {
    let b = value.to_be_bytes();
    format!("{}.{}.{}.{}", b[0], b[1], b[2], b[3])
}

fn compressed_hextets(value: u128) -> String {
    let mut hextets: Vec<String> = (0..8)
        .map(|i| format!("{:x}", (value >> (112 - 16 * i)) & 0xffff))
        .collect();
    let (mut best_start, mut best_len, mut start, mut len) = (0usize, 0usize, None, 0usize);
    for (i, h) in hextets.iter().enumerate() {
        if h == "0" {
            len += 1;
            let s = *start.get_or_insert(i);
            if len > best_len {
                best_len = len;
                best_start = s;
            }
        } else {
            len = 0;
            start = None;
        }
    }
    if best_len > 1 {
        let end = best_start + best_len;
        if end == hextets.len() {
            hextets.push(String::new());
        }
        hextets.splice(best_start..end, [String::new()]);
        if best_start == 0 {
            hextets.insert(0, String::new());
        }
    }
    hextets.join(":")
}

/// `str(IPv6Address)` without scope.
pub fn v6_str(value: u128) -> String {
    if (value >> 32) == 0xffff {
        // _ipv4_mapped_ipv6_to_str: the high hextets, then a dotted quad.
        return format!(
            "{}:{}",
            compressed_hextets(value >> 32),
            v4_str(value as u32)
        );
    }
    compressed_hextets(value)
}

pub fn ip_str(ip: Ip, scope: Option<&str>) -> String {
    match ip {
        Ip::V4(v) => v4_str(v),
        Ip::V6(v) => match scope {
            Some(scope) => format!("{}%{scope}", v6_str(v)),
            None => v6_str(v),
        },
    }
}

fn in_v4(value: u32, net: u32, prefix: u32) -> bool {
    let mask = if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - prefix)
    };
    value & mask == net & mask
}

fn in_v6(value: u128, net: u128, prefix: u32) -> bool {
    let mask = if prefix == 0 {
        0
    } else {
        u128::MAX << (128 - prefix)
    };
    value & mask == net & mask
}

const fn v4(a: u8, b: u8, c: u8, d: u8) -> u32 {
    u32::from_be_bytes([a, b, c, d])
}

const V4_PRIVATE: &[(u32, u32)] = &[
    (v4(0, 0, 0, 0), 8),
    (v4(10, 0, 0, 0), 8),
    (v4(127, 0, 0, 0), 8),
    (v4(169, 254, 0, 0), 16),
    (v4(172, 16, 0, 0), 12),
    (v4(192, 0, 0, 0), 24),
    (v4(192, 0, 0, 170), 31),
    (v4(192, 0, 2, 0), 24),
    (v4(192, 168, 0, 0), 16),
    (v4(198, 18, 0, 0), 15),
    (v4(198, 51, 100, 0), 24),
    (v4(203, 0, 113, 0), 24),
    (v4(240, 0, 0, 0), 4),
    (v4(255, 255, 255, 255), 32),
];
const V4_PRIVATE_EXCEPTIONS: &[(u32, u32)] = &[(v4(192, 0, 0, 9), 32), (v4(192, 0, 0, 10), 32)];

const V6_PRIVATE: &[(u128, u32)] = &[
    (1, 128),
    (0, 128),
    (0xffff_0000_0000, 96),
    (0x0064_ff9b_0001 << 80, 48),
    (0x0100 << 112, 64),
    (0x2001 << 112, 23),
    (0x2001_0db8 << 96, 32),
    (0x2002 << 112, 16),
    (0x3fff << 112, 20),
    (0xfc00 << 112, 7),
    (0xfe80 << 112, 10),
];
const V6_PRIVATE_EXCEPTIONS: &[(u128, u32)] = &[
    ((0x2001_0001 << 96) | 1, 128),
    ((0x2001_0001 << 96) | 2, 128),
    (0x2001_0003 << 96, 32),
    (0x2001_0004_0112 << 80, 48),
    (0x2001_0020 << 96, 28),
    (0x2001_0030 << 96, 28),
];

pub fn v4_is_private(value: u32) -> bool {
    V4_PRIVATE.iter().any(|&(n, p)| in_v4(value, n, p))
        && V4_PRIVATE_EXCEPTIONS
            .iter()
            .all(|&(n, p)| !in_v4(value, n, p))
}

pub fn v4_is_global(value: u32) -> bool {
    !in_v4(value, v4(100, 64, 0, 0), 10) && !v4_is_private(value)
}

pub fn v6_is_global(value: u128) -> bool {
    if (value >> 32) == 0xffff {
        return v4_is_global(value as u32);
    }
    let private = V6_PRIVATE.iter().any(|&(n, p)| in_v6(value, n, p))
        && V6_PRIVATE_EXCEPTIONS
            .iter()
            .all(|&(n, p)| !in_v6(value, n, p));
    !private
}

pub fn ipv4_mapped(value: u128) -> Option<u32> {
    ((value >> 32) == 0xffff).then_some(value as u32)
}

pub fn sixtofour(value: u128) -> Option<u32> {
    ((value >> 112) == 0x2002).then_some((value >> 80) as u32)
}

pub fn is_teredo(value: u128) -> bool {
    (value >> 96) == 0x2001_0000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_parsing_and_formatting() {
        assert!(parse_v4("01.2.3.4").is_err());
        assert!(parse_v4("1.2.3.256").is_err());
        assert_eq!(
            v6_str(parse_v6_value("2001:0db8::0001").unwrap()),
            "2001:db8::1"
        );
        assert_eq!(
            v6_str(parse_v6_value("::ffff:127.0.0.1").unwrap()),
            "::ffff:127.0.0.1"
        );
        assert_eq!(
            v6_str(parse_v6_value("1:0:0:2:0:0:0:3").unwrap()),
            "1:0:0:2::3"
        );
        assert_eq!(
            v6_str(parse_v6_value("1:0:2:3:4:5:6:7").unwrap()),
            "1:0:2:3:4:5:6:7"
        );
        assert!(parse_v6_value("1::2::3").is_err());
        assert!(parse_v6("fe80::1%").is_err());
    }

    #[test]
    fn global_registry() {
        assert!(v4_is_global(parse_v4("8.8.8.8").unwrap()));
        assert!(!v4_is_global(parse_v4("100.64.0.1").unwrap()));
        assert!(v4_is_global(parse_v4("192.0.0.9").unwrap()));
        assert!(!v6_is_global(1));
        assert!(v6_is_global(
            parse_v6_value("2001:4860:4860::8888").unwrap()
        ));
        assert!(!v6_is_global(parse_v6_value("fc00::1").unwrap()));
        assert!(v6_is_global(parse_v6_value("2001:20::1").unwrap()));
    }
}
