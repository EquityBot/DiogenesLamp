"use strict";

// Posting-age estimation. Yupoo assigns album ids from a global sequence,
// so an album's id dates it even when the record carries no date. Anchors
// are (album id, datePublished) pairs measured live on 2026-08-18 from
// album pages spread across the corpus id range. Piecewise-linear
// interpolation between them is accurate to days - plenty for coarse
// "older than N days" filtering. Records scraped after 2026-08 carry a
// real date_published, which always wins over the estimate.
const AGE_ANCHORS = [
  [8348553, Date.UTC(2017, 10, 7)],
  [70347667, Date.UTC(2019, 6, 24)],
  [71628395, Date.UTC(2019, 7, 24)],
  [119598315, Date.UTC(2022, 7, 7)],
  [130174962, Date.UTC(2023, 2, 2)],
  [160926298, Date.UTC(2024, 3, 18)],
  [190225975, Date.UTC(2025, 2, 27)],
  [220478978, Date.UTC(2025, 11, 18)],
  [250756373, Date.UTC(2026, 7, 17)],
];

// Timestamp (ms) an album with this id was most likely published.
// Clamps below the first anchor; extrapolates past the last one at the
// final segment's rate so brand-new albums keep working. Capped at the
// newest instant a Date can hold (see the clamp below): junk ids
// (trillion-scale) would otherwise overflow into Invalid Date, and an
// implied date that far out just means "newer than the table knows". A
// date merely in the future is left alone - it is already never stale.
/**
 * @param {number} id
 */
function estimatePublishedMs(id) {
  const a = AGE_ANCHORS;
  if (id <= a[0][0]) return a[0][1];
  let i = a.findIndex(([anchorId]) => anchorId >= id);
  if (i === -1) i = a.length - 1; // past the last anchor: extrapolate
  const [id0, t0] = a[i - 1];
  const [id1, t1] = a[i];
  // Clamp at the maximum representable Date. age.rs clamps at date.max
  // (year 9999) and this at year ~275760: different bounds, same meaning
  // and same decision - an id implying a date that far out is never stale
  // on either side. What must not happen is the unclamped overflow, which
  // overflowed the scraper's date range and produced an Invalid Date here.
  return Math.min(t0 + ((id - id0) * (t1 - t0)) / (id1 - id0), 8.64e15);
}

// Publication timestamp (ms) for an album record: the scraped
// date_published when present, the id estimate otherwise.
/**
 * @param {{id: string, date_published?: string | null}} album
 */
function publishedMs(album) {
  if (album.date_published) {
    const ts = Date.parse(album.date_published);
    if (!Number.isNaN(ts)) return ts;
  }
  return estimatePublishedMs(Number.parseInt(album.id, 10) || 0);
}

// Whole calendar days between the publish date and today, mirroring
// src/age.rs's whole-day difference. Both sides must count the
// same way: measuring in milliseconds instead would make an album estimated
// exactly N days old pass --max-age-days N at scrape time and then vanish
// from the N-day filter. `today` is the viewer's local date (present day,
// present time), matching the local timestamp the scraper stamps its runs
// with.
const DAY_MS = 86400000;
/** @param {number} nowTs */
function localDayMs(nowTs) {
  const now = new Date(nowTs);
  return Date.UTC(now.getFullYear(), now.getMonth(), now.getDate());
}

/**
 * @param {number} publishedTs
 * @param {number} nowTs
 */
function ageInDays(publishedTs, nowTs) {
  const today = localDayMs(nowTs);
  const publishedDay = Math.floor(publishedTs / DAY_MS) * DAY_MS;
  return Math.round((today - publishedDay) / DAY_MS);
}

// True when the album is within the window, matching is_stale's rule that
// an album exactly max_age_days old is still kept.
/**
 * @param {Parameters<typeof publishedMs>[0]} album
 * @param {number} maxDays
 * @param {number} nowTs
 */
function withinAgeWindow(album, maxDays, nowTs) {
  if (!maxDays) return true;
  return ageInDays(publishedMs(album), nowTs) <= maxDays;
}

if (typeof module !== "undefined") {
  module.exports = { AGE_ANCHORS, estimatePublishedMs, publishedMs,
    DAY_MS, localDayMs, ageInDays, withinAgeWindow };
}
