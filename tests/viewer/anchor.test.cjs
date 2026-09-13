"use strict";

const test = require("node:test");
const assert = require("node:assert");
const { anchorAt, restoredViewTop } = require("../../viewer/anchor.js");

const ROW_H = 312;   // --card-h 300 + --gap 12
const COLS = 5;

test("anchorAt picks the first album of the top visible row", () => {
  // scrolled so row 24 sits 10px above the viewport top
  const spot = anchorAt(24 * ROW_H + 10, ROW_H, COLS, 10000);
  assert.deepStrictEqual(spot, { index: 24 * COLS, offset: 10 });
});

test("anchorAt clamps above the grid instead of floating to row -1", () => {
  // scrollY below spacer.offsetTop: the toolbar is in view, the grid has
  // not started yet. A negative viewTop used to floor to row -1, and the
  // restore then pushed the user a whole row down on every refresh.
  const spot = anchorAt(-150, ROW_H, COLS, 10000);
  assert.deepStrictEqual(spot, { index: 0, offset: 0 });
});

test("anchorAt returns null when there is nothing to anchor on", () => {
  assert.strictEqual(anchorAt(0, ROW_H, COLS, 0), null);
  assert.strictEqual(anchorAt(500, 0, COLS, 10), null);
  assert.strictEqual(anchorAt(500, ROW_H, 0, 10), null);
});

test("anchorAt never indexes past the end of a shrinking list", () => {
  // a filter change can leave fewer albums than the scroll position implies
  const spot = anchorAt(90 * ROW_H, ROW_H, COLS, 12);
  assert.strictEqual(spot.index, 11);
});

test("restoredViewTop keeps the anchored album at the same pixel offset", () => {
  const viewTop = 24 * ROW_H + 10;
  const spot = anchorAt(viewTop, ROW_H, COLS, 10000);
  // 3 sellers' worth of albums (300) land above the anchor
  const moved = restoredViewTop(spot.index + 300, spot.offset, ROW_H, COLS);
  assert.strictEqual(moved, viewTop + (300 / COLS) * ROW_H);
});

test("restoredViewTop is a no-op when the rebuild moved nothing", () => {
  const viewTop = 7 * ROW_H + 44;
  const spot = anchorAt(viewTop, ROW_H, COLS, 10000);
  assert.strictEqual(
    restoredViewTop(spot.index, spot.offset, ROW_H, COLS),
    viewTop,
  );
});

test("anchor round-trip survives a column count change", () => {
  // the narrow breakpoint changes --card-w, so a resize re-renders with
  // fewer columns; the album stays put in row terms, not pixel terms
  const spot = anchorAt(10 * ROW_H, ROW_H, COLS, 10000);
  const narrow = restoredViewTop(spot.index, spot.offset, ROW_H, 2);
  assert.strictEqual(narrow, Math.floor((10 * COLS) / 2) * ROW_H);
});
