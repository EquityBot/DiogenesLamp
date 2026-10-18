//! Estimate an album's posting date from its id.
//!
//! Yupoo assigns album ids from a global sequence, so an id dates an album
//! even before its page is fetched. ANCHORS are (album id, publish date)
//! pairs measured live on 2026-08-18. viewer/age.js carries the same table;
//! a test keeps the two identical.

use crate::isodate::{self, Date};

pub const ANCHORS: [(u128, (i64, i64, i64)); 9] = [
    (8348553, (2017, 11, 7)),
    (70347667, (2019, 7, 24)),
    (71628395, (2019, 8, 24)),
    (119598315, (2022, 8, 7)),
    (130174962, (2023, 3, 2)),
    (160926298, (2024, 4, 18)),
    (190225975, (2025, 3, 27)),
    (220478978, (2025, 12, 18)),
    (250756373, (2026, 8, 17)),
];

fn anchor(index: usize) -> (u128, Date) {
    let (id, (y, m, d)) = ANCHORS[index];
    (id, Date::from_ymd(y, m, d).unwrap_or(Date { ordinal: 1 }))
}

/// Best estimate of when this album was posted: clamped below the first
/// anchor, extrapolated past the last, capped at `date.max`. The offset is
/// floored to match viewer/age.js.
pub fn estimate_published(album_id: u128) -> Date {
    let (first_id, first_date) = anchor(0);
    if album_id <= first_id {
        return first_date;
    }
    let index = ANCHORS
        .iter()
        .position(|&(id, _)| id >= album_id)
        .unwrap_or(ANCHORS.len() - 1);
    let (id0, date0) = anchor(index - 1);
    let (id1, date1) = anchor(index);
    let span_days = (date1.ordinal - date0.ordinal) as u128;
    let offset = (album_id - id0).saturating_mul(span_days) / (id1 - id0);
    let offset = i64::try_from(offset).unwrap_or(i64::MAX);
    date0
        .ordinal
        .checked_add(offset)
        .and_then(Date::from_ordinal)
        .unwrap_or_else(Date::max)
}

/// A previously scraped date wins over the estimate.
pub fn published_date(album_id: u128, known: &str) -> Date {
    if !known.is_empty()
        && let Some(date) = isodate::parse_date(crate::pystr::prefix_chars(known, 10))
    {
        return date;
    }
    estimate_published(album_id)
}

/// True when the album was posted more than `max_age_days` ago; zero or
/// less disables the check.
pub fn is_stale(album_id: u128, max_age_days: i128, today: Date, known: &str) -> bool {
    if max_age_days <= 0 {
        return false;
    }
    i128::from(today.ordinal - published_date(album_id, known).ordinal) > max_age_days
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn junk_ids_cap_at_date_max() {
        assert_eq!(estimate_published(10u128.pow(12)), Date::max());
        assert!(!is_stale(
            10u128.pow(12),
            30,
            Date::from_ymd(2026, 8, 18).unwrap(),
            ""
        ));
    }
}
