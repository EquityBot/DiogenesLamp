"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");
const { createViewState, nextViewState, selectCatalogue } = require("../../viewer/view-state.js");

const albums = [
  { id: "a", title_lc: "blue shirt", seller: "one", category: "shirts", dup_key: "same", links: [], published_ms: 1000 },
  { id: "b", title_lc: "blue shirt", seller: "two", category: "shirts", dup_key: "same", links: [], published_ms: 2000 },
  { id: "c", title_lc: "red shoes", seller: "two", category: "shoes", dup_key: "other", links: [], published_ms: 3000 },
];
const catalogue = { albums, favorites: new Set(["a", "c"]), generatedAt: "today", bannedKeys: new Set() };

test("removed saved options clear filters so the available catalogue stays visible", () => {
  let view = createViewState({ seller: "removed", category: "removed" });
  view = nextViewState(view, { type: "restore-options", options: {
    seller: ["", "one", "two"], category: ["", "shirts", "shoes"],
  } });
  assert.equal(view.filters.seller, "");
  assert.equal(view.filters.category, "");
  assert.equal(selectCatalogue(view, catalogue).filtered.length, 3);
});

test("late options restore saved choices until the user changes a filter", () => {
  let view = createViewState({ seller: "two", category: "shirts" }, {});
  view = nextViewState(view, { type: "restore-options", options: { seller: ["", "two"], category: [""] } });
  assert.deepEqual(selectCatalogue(view, catalogue).filtered.map(a => a.id), ["c", "b"]);
  assert.equal(view.filters.category, "");
  view = nextViewState(view, { type: "filter", filters: { search: "red" } });
  view = nextViewState(view, { type: "restore-options", options: { category: ["", "shirts"] } });
  assert.equal(view.filters.category, "");
  assert.deepEqual(selectCatalogue(view, catalogue).filtered.map(a => a.id), ["c"]);
});

test("group entry ignores filters, then exit restores choices and scroll position", () => {
  let view = createViewState({ search: "red" });
  view = nextViewState(view, { type: "cursor", index: 20, count: 30 });
  view = nextViewState(view, { type: "open-group", groupKey: "same", scrollY: 1450 });
  const group = selectCatalogue(view, catalogue);
  assert.deepEqual(group.filtered.map(a => a.id), ["b", "a"]);
  assert.equal(group.cursor, 1);
  view = nextViewState(view, { type: "close-group" });
  assert.equal(view.preGroupScroll, 1450);
  assert.equal(view.cursor, 0);
  assert.deepEqual(selectCatalogue(view, catalogue).filtered.map(a => a.id), ["c"]);
});

test("removing favourites clamps the cursor even without a filter event", () => {
  let view = createViewState({ favsOnly: true });
  view = nextViewState(view, { type: "cursor", index: 1, count: 2 });
  const remaining = selectCatalogue(view, { ...catalogue, favorites: new Set(["a"]) });
  assert.deepEqual(remaining.filtered.map(a => a.id), ["a"]);
  assert.equal(remaining.cursor, 0);
  const empty = selectCatalogue(view, { ...catalogue, favorites: new Set() });
  assert.deepEqual(empty.filtered, []);
  assert.equal(empty.cursor, 0);
});

test("clearing filters exits a group and prevents late restoration", () => {
  let view = createViewState({ seller: "two", hideDups: true });
  view = nextViewState(view, { type: "open-group", groupKey: "same", scrollY: 20 });
  view = nextViewState(view, { type: "clear-filters" });
  view = nextViewState(view, { type: "restore-options", options: { seller: ["two"] } });
  assert.equal(view.groupKey, null);
  assert.equal(view.filters.seller, "");
  assert.equal(view.filters.hideDups, false);
  assert.equal(selectCatalogue(view, catalogue).filtered.length, 3);
});

test("duplicate collapse keeps counts and respects corpus-wide exclusions", () => {
  const view = createViewState({ hideDups: true });
  const collapsed = selectCatalogue(view, catalogue);
  assert.equal(collapsed.filtered.length, 2);
  assert.equal(collapsed.dupCounts.get("same"), 2);
  assert.equal(collapsed.dupHidden, 1);
  assert.equal(selectCatalogue(view, { ...catalogue, bannedKeys: new Set(["same"]) }).filtered.length, 3);
});

test("age and new-only filters use the supplied catalogue date and clock", () => {
  const now = 10 * 86400000;
  const items = [
    { ...albums[0], first_seen: "today", published_ms: now - 86400000 },
    { ...albums[1], first_seen: "yesterday", published_ms: now },
    { ...albums[2], first_seen: "today", published_ms: now - 3 * 86400000 },
  ];
  const view = createViewState({ age: "1", newOnly: true });
  assert.deepEqual(selectCatalogue(view, { ...catalogue, albums: items }, now).filtered.map(a => a.id), ["a"]);
});
