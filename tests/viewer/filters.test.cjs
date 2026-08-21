"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");
const path = require("node:path");

const { FILTER_STORAGE_KEY, defaultFilters, normalizeFilters, anyFilterActive,
  serializeFilters, readFilters, writeFilters, matchSearch } =
  require(path.join(__dirname, "..", "..", "viewer", "filters.js"));

// Minimal stand-in for localStorage. `fail` makes both accessors throw, which
// is what a browser does when storage is disabled or the quota is full.
function fakeStorage(initial = {}, fail = false) {
  const data = { ...initial };
  return {
    data,
    getItem(key) {
      if (fail) throw new Error("storage disabled");
      return Object.prototype.hasOwnProperty.call(data, key) ? data[key] : null;
    },
    setItem(key, value) {
      if (fail) throw new Error("quota exceeded");
      data[key] = String(value);
    },
  };
}

test("defaults describe an unfiltered view", () => {
  const defaults = defaultFilters();
  assert.equal(anyFilterActive(defaults), false);
  assert.deepEqual(serializeFilters(defaults), {});
});

test("each filter on its own counts as active", () => {
  for (const [key, value] of Object.entries({
    search: "shoe", seller: "someseller", category: "hoodies", age: "90",
    newOnly: true, favsOnly: true, hideDups: true,
  })) {
    const filters = { ...defaultFilters(), [key]: value };
    assert.equal(anyFilterActive(filters), true, `${key} should count as active`);
    assert.deepEqual(serializeFilters(filters), { [key]: value });
  }
});

test("a wrong-typed stored value falls back rather than being trusted", () => {
  // The blob is hand-editable. "false" is a non-empty string, so assigning it
  // straight to .checked would tick the box.
  const restored = normalizeFilters({ hideDups: "false", search: 42, age: null });
  assert.equal(restored.hideDups, false);
  assert.equal(restored.search, "");
  assert.equal(restored.age, "");
});

test("normalize tolerates junk instead of throwing", () => {
  for (const junk of [null, undefined, 7, "text", []]) {
    assert.deepEqual(normalizeFilters(junk), defaultFilters());
  }
});

test("a blob from an older build leaves unknown fields at their default", () => {
  const restored = normalizeFilters({ search: "kept" });
  assert.equal(restored.search, "kept");
  assert.equal(restored.hideDups, false);
  assert.equal(restored.age, "");
});

test("write then read round-trips the active filters", () => {
  const storage = fakeStorage();
  const filters = { ...defaultFilters(), search: "tee", age: "180", favsOnly: true };
  writeFilters(storage, filters);
  assert.deepEqual(readFilters(storage), filters);
  // Defaults are not stored, so the blob stays small.
  assert.deepEqual(JSON.parse(storage.data[FILTER_STORAGE_KEY]),
    { search: "tee", age: "180", favsOnly: true });
});

test("unreadable storage yields defaults instead of throwing", () => {
  // This runs during start-up: an exception here binds no listeners and
  // leaves a blank page.
  assert.deepEqual(readFilters(fakeStorage({}, true)), defaultFilters());
  assert.deepEqual(readFilters(fakeStorage({ [FILTER_STORAGE_KEY]: "{not json" })),
    defaultFilters());
  assert.deepEqual(readFilters(fakeStorage({ [FILTER_STORAGE_KEY]: "null" })),
    defaultFilters());
});

test("a failed write is swallowed", () => {
  assert.doesNotThrow(() => writeFilters(fakeStorage({}, true), defaultFilters()));
});

test("matchSearch supports multi-term whitespace-separated matching in any order", () => {
  const title = "Nike Tech Fleece Black Zip Hoodie 2026";
  assert.equal(matchSearch(title, ""), true);
  assert.equal(matchSearch(title, "   "), true);
  assert.equal(matchSearch(title, "nike"), true);
  assert.equal(matchSearch(title, "NIKE HOODIE"), true);
  assert.equal(matchSearch(title, "hoodie black 2026"), true);
  assert.equal(matchSearch(title, "fleece zip nike"), true);
  assert.equal(matchSearch(title, "adidas"), false);
  assert.equal(matchSearch(title, "nike adidas"), false);
});
