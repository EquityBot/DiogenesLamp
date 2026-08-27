"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const {
  createViewState,
  hasViewStateParams,
  nextViewState,
  urlForViewState,
} = require(path.join(__dirname, "..", "..", "viewer", "view-state.js"));

test("createViewState normalises persisted filters", () => {
  assert.deepEqual(createViewState({ search: "shoe", newOnly: true, hideDups: "yes" }), {
    filters: {
      search: "shoe", seller: "", category: "", age: "", sort: "default",
      newOnly: true, favsOnly: false, hideDups: false,
    },
    groupKey: null,
    preGroupScroll: 0,
    cursor: 0,
    shopsOpen: false,
  });
});

test("filter and search events leave one canonical filter state", () => {
  const grouped = {
    ...createViewState({ seller: "alpha", newOnly: true }),
    groupKey: "i:weidian:7",
    preGroupScroll: 480,
    cursor: 9,
  };
  const filtered = nextViewState(grouped, {
    type: "filter",
    filters: { ...grouped.filters, category: "jackets", hideDups: true },
  });
  assert.equal(filtered.filters.seller, "alpha");
  assert.equal(filtered.filters.category, "jackets");
  assert.equal(filtered.filters.hideDups, true);
  assert.equal(filtered.groupKey, null);
  assert.equal(filtered.cursor, 0);

  const searched = nextViewState(filtered, { type: "search", value: "  red shoe  " });
  assert.equal(searched.filters.search, "  red shoe  ");
  assert.equal(searched.filters.category, "jackets");
  assert.equal(searched.groupKey, null);
});

test("clear filters resets every persisted field and closes a group", () => {
  const state = {
    ...createViewState({ search: "shoe", sort: "price-asc", favsOnly: true }),
    groupKey: "t:a:shoe",
    cursor: 4,
  };
  assert.deepEqual(nextViewState(state, { type: "clear-filters" }), createViewState());
});

test("group transitions capture and restore the browse context", () => {
  const initial = { ...createViewState(), cursor: 12 };
  const opened = nextViewState(initial, {
    type: "open-group",
    groupKey: "i:weidian:7",
    scrollY: 640,
  });
  assert.equal(opened.groupKey, "i:weidian:7");
  assert.equal(opened.preGroupScroll, 640);
  assert.equal(opened.cursor, 12);

  const closed = nextViewState(opened, { type: "close-group" });
  assert.equal(closed.groupKey, null);
  assert.equal(closed.preGroupScroll, 640);
  assert.equal(closed.cursor, 0);
});

test("Escape closes the innermost overlay before the duplicate group", () => {
  const both = {
    ...createViewState(),
    groupKey: "i:weidian:7",
    shopsOpen: true,
    cursor: 3,
  };
  const panelClosed = nextViewState(both, { type: "escape" });
  assert.equal(panelClosed.shopsOpen, false);
  assert.equal(panelClosed.groupKey, "i:weidian:7");
  assert.equal(panelClosed.cursor, 3);

  const groupClosed = nextViewState(panelClosed, { type: "escape" });
  assert.equal(groupClosed.groupKey, null);
  assert.equal(groupClosed.cursor, 0);
});

test("URL state round-trips filters and grouping while preserving unrelated parameters", () => {
  const state = {
    ...createViewState({
      search: "red shoe", seller: "alpha", category: "jackets", age: "90",
      sort: "price-asc", newOnly: true, favsOnly: true, hideDups: true,
    }),
    groupKey: "i:weidian:7",
  };
  const url = urlForViewState("http://localhost:8000/?fixture=demo#grid", state);
  assert.equal(url.searchParams.get("fixture"), "demo");
  assert.equal(url.hash, "#grid");
  assert.equal(hasViewStateParams(url), true);

  const restored = nextViewState(
    { ...createViewState(), shopsOpen: true, cursor: 22 },
    { type: "url", url },
  );
  assert.deepEqual(restored.filters, state.filters);
  assert.equal(restored.groupKey, "i:weidian:7");
  assert.equal(restored.shopsOpen, false);
  assert.equal(restored.cursor, 0);
});

test("a URL without viewer parameters restores the unfiltered catalogue", () => {
  const active = createViewState({ search: "shoe", newOnly: true });
  const restored = nextViewState(active, {
    type: "url",
    url: "http://localhost:8000/?fixture=demo",
  });
  assert.deepEqual(restored, createViewState());
  assert.equal(hasViewStateParams("http://localhost:8000/?fixture=demo"), false);
});

test("unknown events leave the state untouched", () => {
  const state = createViewState({ search: "shoe" });
  assert.equal(nextViewState(state, { type: "future-event" }), state);
});

test("app delegates stable view transitions instead of retaining parallel state", () => {
  const app = fs.readFileSync(path.join(__dirname, "..", "..", "viewer", "app.js"), "utf8");
  assert.match(app, /nextViewState\(/);
  for (const oldPath of ["state.dupGroup", "state.preGroupScroll", "state.cursor"]) {
    assert.equal(app.includes(oldPath), false, `${oldPath} still bypasses the view reducer`);
  }
});

test("the transition module stays independent of browser effects and loads in order", () => {
  const root = path.join(__dirname, "..", "..");
  const source = fs.readFileSync(path.join(root, "viewer", "view-state.js"), "utf8");
  for (const browserEffect of ["document.", "window.", "localStorage", "history."]) {
    assert.equal(source.includes(browserEffect), false, `${browserEffect} leaked into pure state`);
  }

  const html = fs.readFileSync(path.join(root, "viewer", "index.html"), "utf8");
  assert.ok(html.indexOf('src="filters.js"') < html.indexOf('src="view-state.js"'));
  assert.ok(html.indexOf('src="view-state.js"') < html.indexOf('src="app.js"'));
});
