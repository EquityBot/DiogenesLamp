"use strict";

// Query marking and grid movement. Pure functions only, no DOM access, so the
// same file runs in the browser (script tag before app.js) and under node for
// tests. app.js supplies the measurements.
//
// Ordering lives in filters.js with the rest of the filter state; this file
// only marks what the query hit and moves the cursor over the result.

// Where the query hit the title, as [start, end) pairs over the original
// string. The caller strikes those spans in the second ink, so a matched
// title says which part of it matched rather than only that it did.
//
// The query is multi-term and order-free, exactly as matchSearch reads it, so
// every term is marked wherever it lands. Matching is done on lowercased
// copies of both sides and the offsets are reported against the original, so
// the marks fall on the real characters.
function matchRanges(title, query) {
  const text = (title || "").toLowerCase();
  const terms = (query || "").trim().toLowerCase().split(/\s+/).filter(Boolean);
  if (!terms.length || !text) return [];
  const hits = [];
  for (const term of terms) {
    let from = 0;
    for (;;) {
      const at = text.indexOf(term, from);
      if (at < 0) break;
      hits.push([at, at + term.length]);
      from = at + term.length;
      // A cap keeps a one-character term on a long title from producing a
      // mark per character, which costs more than it tells anyone.
      if (hits.length >= 40) break;
    }
    if (hits.length >= 40) break;
  }
  return mergeRanges(hits);
}

// Terms overlap: "air jordan" against "air jordan" marks 0-3 and 4-10, but
// "jord ordan" would mark 4-8 and 5-10, and two <mark> elements cannot share a
// character. Sort, then fold anything that touches or overlaps into one span.
function mergeRanges(ranges) {
  if (ranges.length < 2) return ranges;
  const sorted = ranges.slice().sort((a, b) => a[0] - b[0] || a[1] - b[1]);
  const out = [sorted[0]];
  for (const [start, end] of sorted.slice(1)) {
    const last = out[out.length - 1];
    if (start <= last[1]) last[1] = Math.max(last[1], end);
    else out.push([start, end]);
  }
  return out;
}

// How far a Page move travels, measured in albums - the unit gridStep
// counts in. A screenful is the rows that fit times the columns in each;
// handing it the row count alone makes a page shorter than a single
// ArrowDown on any grid with more columns than rows on screen, which is
// every desktop layout. app.js supplies the measurements.
function pageStep(viewportHeight, rowHeight, cols) {
  const rows = Math.max(1, Math.floor(viewportHeight / rowHeight));
  return rows * Math.max(1, cols);
}

// Where the cursor sits after a pass that rebuilt the filtered list. The
// single tab stop in the lattice is stamped on the card whose index is the
// cursor, so a list that shrank past it would carry no stop at all and the
// keyboard could not enter the grid again.
function clampCursor(cursor, count) {
  if (count <= 0) return 0;
  return Math.min(Math.max(cursor, 0), count - 1);
}

// Whether the window already drawn is still the window that should be drawn.
// renderGrid skips its pass when this holds, which is what keeps a scroll
// step from re-walking a window nothing changed in.
//
// The cursor is part of the comparison because a skipped render is a render
// that does not stamp the roving tab stop. An ArrowRight inside the visible
// row moves the cursor and leaves the row range, the column count, the
// filtered length and the filter version all identical, so without the
// cursor here the pass was skipped and tabindex="0" stayed on the card the
// user had just left - Tab then jumped backwards to it, which is the whole
// behaviour the roving tabindex exists to provide. app.js supplies the
// measurements; the comparison lives here so it can be tested without a DOM.
const RENDER_WINDOW_KEYS = ["start", "end", "cols", "count", "version", "cursor"];

function sameRenderWindow(drawn, wanted) {
  if (!drawn || !wanted) return false;
  return RENDER_WINDOW_KEYS.every((key) => drawn[key] === wanted[key]);
}

// Next index for a keyboard move across the lattice. Returns the same index
// when the move would leave the catalogue, so holding an arrow at the edge
// parks rather than wrapping to the far side, which loses the reader's place.
//
// Down and up move a whole row; a down move from the last partial row lands
// on the final album rather than refusing, because the row below exists on
// screen even when it is not full.
function gridStep(index, key, cols, count, perPage) {
  if (count <= 0 || cols <= 0) return 0;
  const here = Math.min(Math.max(index, 0), count - 1);
  const page = Math.max(1, perPage || cols);
  let next = here;
  if (key === "ArrowRight") next = here + 1;
  else if (key === "ArrowLeft") next = here - 1;
  else if (key === "ArrowDown") next = here + cols >= count ? count - 1 : here + cols;
  else if (key === "ArrowUp") next = here - cols;
  else if (key === "PageDown") next = Math.min(count - 1, here + page);
  else if (key === "PageUp") next = Math.max(0, here - page);
  else if (key === "Home") next = 0;
  else if (key === "End") next = count - 1;
  else return here;
  if (next < 0 || next >= count) return here;
  return next;
}

if (typeof module !== "undefined") {
  module.exports = { matchRanges, mergeRanges, pageStep, clampCursor, gridStep,
    RENDER_WINDOW_KEYS, sameRenderWindow };
}
