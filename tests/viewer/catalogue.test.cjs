"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");
const path = require("node:path");

const { matchRanges, mergeRanges, pageStep, clampCursor, gridStep } =
  require(path.join(__dirname, "..", "..", "viewer", "catalogue.js"));

test("no query marks nothing", () => {
  assert.deepEqual(matchRanges("Air Jordan 1", ""), []);
  assert.deepEqual(matchRanges("Air Jordan 1", "   "), []);
  assert.deepEqual(matchRanges("", "jordan"), []);
});

test("marks land on the original characters, case regardless", () => {
  const title = "Air JORDAN 1 jordan";
  const ranges = matchRanges(title, "Jordan");
  assert.deepEqual(ranges, [[4, 10], [13, 19]]);
  assert.equal(title.slice(4, 10), "JORDAN");
  assert.equal(title.slice(13, 19), "jordan");
});

test("every term of a multi-term query is marked, in any order", () => {
  // matchSearch admits the album when all terms appear anywhere, so the marks
  // have to follow the same rule or a matched card shows no mark.
  const title = "black nylon jacket";
  assert.deepEqual(matchRanges(title, "jacket black"), [[0, 5], [12, 18]]);
});

test("a term that matches nothing costs the others nothing", () => {
  assert.deepEqual(matchRanges("black nylon jacket", "black zzz"), [[0, 5]]);
});

test("overlapping terms fold into one span", () => {
  // Two <mark> elements cannot share a character.
  assert.deepEqual(matchRanges("jordan", "jord ordan"), [[0, 6]]);
  assert.deepEqual(matchRanges("aaaa", "aa"), [[0, 4]]);
});

test("mergeRanges leaves separated spans alone and folds touching ones", () => {
  assert.deepEqual(mergeRanges([[0, 2], [5, 7]]), [[0, 2], [5, 7]]);
  assert.deepEqual(mergeRanges([[5, 7], [0, 2]]), [[0, 2], [5, 7]]);
  assert.deepEqual(mergeRanges([[0, 3], [3, 6]]), [[0, 6]]);
  assert.deepEqual(mergeRanges([]), []);
});

test("a one-character term stops marking rather than marking everything", () => {
  assert.equal(matchRanges("a".repeat(200), "a").length, 1);
  assert.ok(matchRanges("ab".repeat(200), "a").length <= 40);
});

test("arrows move by one and by a row, and park at the edges", () => {
  // 10 albums, 4 columns: rows are 0-3, 4-7, 8-9.
  assert.equal(gridStep(0, "ArrowRight", 4, 10), 1);
  assert.equal(gridStep(0, "ArrowLeft", 4, 10), 0);
  assert.equal(gridStep(0, "ArrowUp", 4, 10), 0);
  assert.equal(gridStep(0, "ArrowDown", 4, 10), 4);
  assert.equal(gridStep(9, "ArrowRight", 4, 10), 9);
});

test("a move down from the last full row lands on the last album", () => {
  // Album 7 sits above nothing: the row below holds only 8 and 9.
  assert.equal(gridStep(7, "ArrowDown", 4, 10), 9);
  assert.equal(gridStep(9, "ArrowDown", 4, 10), 9);
});

test("home, end and page moves stay inside the catalogue", () => {
  assert.equal(gridStep(5, "Home", 4, 10), 0);
  assert.equal(gridStep(5, "End", 4, 10), 9);
  assert.equal(gridStep(0, "PageDown", 4, 10, 8), 8);
  assert.equal(gridStep(0, "PageDown", 4, 10, 40), 9);
  assert.equal(gridStep(9, "PageUp", 4, 10, 8), 1);
  assert.equal(gridStep(1, "PageUp", 4, 10, 8), 0);
});

test("a page move travels a screenful of albums, not a screenful of rows", () => {
  // gridStep counts in albums. --card-h 288 + --gap 10 is a 298px row pitch,
  // so a 1080px viewport holds 3 rows - 18 albums across 6 columns. Passing
  // the row count instead moved 3, half a row, which is less than a single
  // ArrowDown, and it is wrong for every grid with more columns than rows.
  assert.equal(pageStep(1080, 298, 6), 18);
  assert.equal(gridStep(0, "PageDown", 6, 1000, pageStep(1080, 298, 6)), 18);
  assert.ok(pageStep(1080, 298, 6) > gridStep(0, "ArrowDown", 6, 1000));
});

test("a page is never shorter than one row, however small the viewport", () => {
  assert.equal(pageStep(100, 298, 6), 6);   // viewport shorter than a card
  assert.equal(pageStep(1080, 298, 1), 3);  // one column: rows are albums
});

test("a cursor stranded past a shrunken list is pulled back onto the last card", () => {
  // renderGrid stamps the grid's only tab stop on the card whose index is
  // the cursor. Left past the end, no card carries it and the keyboard
  // cannot enter the grid at all.
  assert.equal(clampCursor(499, 3), 2);  // a x-N badge opened from deep in the lattice
  assert.equal(clampCursor(1, 1), 0);    // the last favourite un-starred in a favs view
  assert.equal(clampCursor(0, 0), 0);    // nothing matched: nothing to park on
  assert.equal(clampCursor(4, 10), 4);   // a cursor already inside is left alone
  assert.equal(clampCursor(-3, 10), 0);
});

test("a key the lattice does not own leaves the cursor alone", () => {
  assert.equal(gridStep(3, "Tab", 4, 10), 3);
  assert.equal(gridStep(3, "a", 4, 10), 3);
});

test("an empty or unmeasured grid answers with the first index", () => {
  assert.equal(gridStep(5, "ArrowDown", 4, 0), 0);
  assert.equal(gridStep(5, "ArrowDown", 0, 10), 0);
});

test("a cursor left past the end by a shrinking filter is pulled back in", () => {
  assert.equal(gridStep(99, "ArrowLeft", 4, 10), 8);
  assert.equal(gridStep(-5, "ArrowRight", 4, 10), 1);
});
