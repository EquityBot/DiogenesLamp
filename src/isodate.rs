//! Proleptic Gregorian dates and the `date.fromisoformat` /
//! `datetime.fromisoformat` grammar of CPython's C implementation.

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date {
    /// `date.toordinal()`: 0001-01-01 is 1.
    pub ordinal: i64,
}

pub const MAX_ORDINAL: i64 = 3_652_059; // 9999-12-31

fn is_leap(year: i64) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if is_leap(year) => 29,
        _ => 28,
    }
}

fn days_before_year(year: i64) -> i64 {
    let y = year - 1;
    y * 365 + y / 4 - y / 100 + y / 400
}

fn days_before_month(year: i64, month: i64) -> i64 {
    (1..month).map(|m| days_in_month(year, m)).sum()
}

impl Date {
    pub fn from_ymd(year: i64, month: i64, day: i64) -> Option<Date> {
        if !(1..=9999).contains(&year) || !(1..=12).contains(&month) {
            return None;
        }
        if day < 1 || day > days_in_month(year, month) {
            return None;
        }
        Some(Date {
            ordinal: days_before_year(year) + days_before_month(year, month) + day,
        })
    }

    pub fn from_ordinal(ordinal: i64) -> Option<Date> {
        (1..=MAX_ORDINAL)
            .contains(&ordinal)
            .then_some(Date { ordinal })
    }

    pub fn max() -> Date {
        Date {
            ordinal: MAX_ORDINAL,
        }
    }

    pub fn ymd(self) -> (i64, i64, i64) {
        let mut year = (self.ordinal * 400 / 146_097).max(1);
        while days_before_year(year + 1) < self.ordinal {
            year += 1;
        }
        while days_before_year(year) >= self.ordinal {
            year -= 1;
        }
        let mut day = self.ordinal - days_before_year(year);
        let mut month = 1;
        while day > days_in_month(year, month) {
            day -= days_in_month(year, month);
            month += 1;
        }
        (year, month, day)
    }

    pub fn isoformat(self) -> String {
        let (y, m, d) = self.ymd();
        format!("{y:04}-{m:02}-{d:02}")
    }

    /// ISO weekday, Monday = 1.
    fn isoweekday(self) -> i64 {
        (self.ordinal + 6) % 7 + 1
    }
}

fn digits(s: &str) -> Option<i64> {
    (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
        .then(|| s.parse().ok())
        .flatten()
}

fn isoweek_to_gregorian(year: i64, week: i64, day: i64) -> Option<Date> {
    if !(1..=9999).contains(&year) {
        return None;
    }
    if !(1..53).contains(&week) {
        let mut out_of_range = true;
        if week == 53 {
            // A year has 53 weeks if Jan 1 is a Thursday, or a Wednesday in a leap year.
            let first_weekday = Date::from_ymd(year, 1, 1)?.ordinal % 7;
            if first_weekday == 4 || (first_weekday == 3 && is_leap(year)) {
                out_of_range = false;
            }
        }
        if out_of_range {
            return None;
        }
    }
    if !(1..=7).contains(&day) {
        return None;
    }
    let jan4 = Date::from_ymd(year, 1, 4)?;
    let week1_monday = jan4.ordinal - (jan4.isoweekday() - 1);
    Date::from_ordinal(week1_monday + (week - 1) * 7 + (day - 1))
}

fn char_at(s: &str, i: usize) -> Option<char> {
    s.chars().nth(i)
}

fn slice(s: &str, start: usize, end: usize) -> String {
    s.chars()
        .skip(start)
        .take(end.saturating_sub(start))
        .collect()
}

/// `_parse_isoformat_date` on a 7, 8 or 10 character string.
fn parse_date_part(s: &str) -> Option<Date> {
    let len = s.chars().count();
    if !matches!(len, 7 | 8 | 10) {
        return None;
    }
    let year = digits(&slice(s, 0, 4))?;
    let has_sep = char_at(s, 4) == Some('-');
    let mut pos = 4 + usize::from(has_sep);
    if char_at(s, pos) == Some('W') {
        pos += 1;
        let week = digits(&slice(s, pos, pos + 2))?;
        pos += 2;
        let mut day = 1;
        if len > pos {
            if (char_at(s, pos) == Some('-')) != has_sep {
                return None;
            }
            pos += usize::from(has_sep);
            day = digits(&slice(s, pos, pos + 1))?;
        }
        return isoweek_to_gregorian(year, week, day);
    }
    let month = digits(&slice(s, pos, pos + 2))?;
    pos += 2;
    if (char_at(s, pos) == Some('-')) != has_sep {
        return None;
    }
    pos += usize::from(has_sep);
    let day_text = slice(s, pos, pos + 2);
    if day_text.chars().count() != 2 {
        return None;
    }
    Date::from_ymd(year, month, digits(&day_text)?)
}

/// `date.fromisoformat(s)`.
pub fn parse_date(s: &str) -> Option<Date> {
    parse_date_part(s)
}

fn parse_hh_mm_ss_ff(t: &str) -> Option<[i64; 4]> {
    let chars: Vec<char> = t.chars().collect();
    let len = chars.len();
    let mut comps = [0i64; 4];
    let mut pos = 0;
    let mut has_sep = false;
    for (comp, slot) in comps.iter_mut().enumerate().take(3) {
        if len - pos < 2 {
            return None;
        }
        let text: String = chars[pos..pos + 2].iter().collect();
        *slot = digits(&text)?;
        pos += 2;
        let next = chars.get(pos).copied();
        if comp == 0 {
            has_sep = next == Some(':');
        }
        if next.is_none() || comp >= 2 {
            break;
        }
        if has_sep && next != Some(':') {
            return None;
        }
        pos += usize::from(has_sep);
    }
    if pos < len {
        if chars[pos] != '.' && chars[pos] != ',' {
            return None;
        }
        pos += 1;
        let rest: String = chars[pos..].iter().collect();
        if rest.is_empty() || !rest.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let take = rest.len().min(6);
        let mut micro: i64 = rest[..take].parse().ok()?;
        for _ in take..6 {
            micro *= 10;
        }
        comps[3] = micro;
    }
    Some(comps)
}

fn separator_location(s: &[char]) -> Option<usize> {
    let len = s.len();
    if len == 7 {
        return Some(7);
    }
    let digit = |i: usize| s.get(i).is_some_and(char::is_ascii_digit);
    if s[4] == '-' {
        if s[5] == 'W' {
            if len > 8 && s[8] == '-' {
                if len == 9 {
                    return None;
                }
                if len > 10 && digit(10) {
                    return Some(8);
                }
                return Some(10);
            }
            return Some(8);
        }
        return Some(10);
    }
    if s[4] == 'W' {
        let mut idx = 7;
        while idx < len && digit(idx) {
            idx += 1;
        }
        if idx < 9 {
            return Some(idx);
        }
        return Some(if idx % 2 == 0 { 7 } else { 8 });
    }
    Some(8)
}

/// `datetime.fromisoformat(s)` as microseconds since 0001-01-01T00:00 in
/// local wall time, plus the UTC offset in microseconds when one is given.
pub fn parse_datetime(s: &str) -> Option<(i64, Option<i64>)> {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() < 7 {
        return None;
    }
    let sep = separator_location(&chars)?;
    let date_text: String = chars[..sep.min(chars.len())].iter().collect();
    let date = parse_date_part(&date_text)?;
    let time_text: String = chars
        .get(sep + 1..)
        .map(|c| c.iter().collect())
        .unwrap_or_default();
    let mut day_micros = 0i64;
    let mut offset = None;
    let mut ordinal = date.ordinal;
    if !time_text.is_empty() {
        let t: Vec<char> = time_text.chars().collect();
        if t.len() < 2 {
            return None;
        }
        let find = |c: char| t.iter().position(|&x| x == c).map(|p| p + 1);
        let tz_pos = find('-')
            .or_else(|| find('+'))
            .or_else(|| find('Z'))
            .unwrap_or(0);
        let main: String = if tz_pos > 0 {
            t[..tz_pos - 1].iter().collect()
        } else {
            time_text.clone()
        };
        let [mut hour, minute, second, micro] = parse_hh_mm_ss_ff(&main)?;
        if hour == 24 {
            if minute == 0 && second == 0 && micro == 0 {
                hour = 0;
                ordinal += 1;
            } else {
                return None;
            }
        }
        if hour > 23 || minute > 59 || second > 59 {
            return None;
        }
        if tz_pos > 0 {
            if tz_pos == t.len() && t[t.len() - 1] == 'Z' {
                offset = Some(0);
            } else {
                let tz: String = t[tz_pos..].iter().collect();
                if matches!(tz.chars().count(), 0 | 1 | 3) || t[tz_pos - 1] == 'Z' {
                    return None;
                }
                let [h, m, sec, us] = parse_hh_mm_ss_ff(&tz)?;
                if h > 23 || m > 59 || sec > 59 {
                    return None;
                }
                let sign = if t[tz_pos - 1] == '-' { -1 } else { 1 };
                offset = Some(sign * (((h * 60 + m) * 60 + sec) * 1_000_000 + us));
            }
        }
        day_micros = ((hour * 60 + minute) * 60 + second) * 1_000_000 + micro;
    }
    Date::from_ordinal(ordinal)?;
    Some((ordinal * 86_400_000_000 + day_micros, offset))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinals_and_formats() {
        assert_eq!(Date::from_ymd(1, 1, 1).unwrap().ordinal, 1);
        assert_eq!(Date::max().isoformat(), "9999-12-31");
        assert_eq!(
            Date::from_ymd(2026, 8, 17).unwrap().isoformat(),
            "2026-08-17"
        );
        assert_eq!(parse_date("20260801"), Date::from_ymd(2026, 8, 1));
        assert_eq!(parse_date("2026010100"), Date::from_ymd(2026, 1, 1));
        assert_eq!(parse_date("2020-W53-7"), Date::from_ymd(2021, 1, 3));
        assert_eq!(parse_date("2026-W01-1"), Date::from_ymd(2025, 12, 29));
        assert_eq!(parse_date("2026-02-30"), None);
        assert_eq!(parse_date("0000-02-01"), None);
        assert!(parse_datetime("2026-08-01T00:00:00+10:00").is_some());
        assert!(parse_datetime("2026-08-01 10:00Z").is_some());
        assert!(parse_datetime("").is_none());
    }
}
