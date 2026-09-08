"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");
const path = require("node:path");

const { FILTER_STORAGE_KEY, defaultFilters, normalizeFilters, anyFilterActive,
  serializeFilters, readStored, writeStored, readFilters, writeFilters,
  matchSearch, sortAlbums, formatPrice, isSearchShortcut,
  favouritesExportPlan } =
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

// The helpers take the host, not the Storage, so a plain window stands in.
function fakeHost(storage) {
  return { localStorage: storage };
}

// A host whose localStorage *getter* throws, which is what a browser with
// site data blocked for the origin actually does: the SecurityError comes
// from reaching for the Storage, before getItem exists to be called. A guard
// that takes the Storage as an argument never sees this - the argument is
// evaluated at the call site, outside the guard's try.
function blockedHost() {
  const host = {};
  Object.defineProperty(host, "localStorage", {
    get() {
      const err = new Error("Access is denied for this document.");
      err.name = "SecurityError";
      throw err;
    },
  });
  return host;
}

test("defaults describe an unfiltered view", () => {
  const defaults = defaultFilters();
  assert.equal(anyFilterActive(defaults), false);
  assert.deepEqual(serializeFilters(defaults), {});
});

test("each filter on its own counts as active", () => {
  for (const [key, value] of Object.entries({
    search: "shoe", seller: "someseller", category: "hoodies", age: "90",
    sort: "price-asc", newOnly: true, favsOnly: true, hideDups: true,
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
  writeFilters(fakeHost(storage), filters);
  assert.deepEqual(readFilters(fakeHost(storage)), filters);
  // Defaults are not stored, so the blob stays small.
  assert.deepEqual(JSON.parse(storage.data[FILTER_STORAGE_KEY]),
    { search: "tee", age: "180", favsOnly: true });
});

test("unreadable storage yields defaults instead of throwing", () => {
  // This runs during start-up: an exception here binds no listeners and
  // leaves a blank page.
  assert.deepEqual(readFilters(fakeHost(fakeStorage({}, true))), defaultFilters());
  assert.deepEqual(
    readFilters(fakeHost(fakeStorage({ [FILTER_STORAGE_KEY]: "{not json" }))),
    defaultFilters());
  assert.deepEqual(
    readFilters(fakeHost(fakeStorage({ [FILTER_STORAGE_KEY]: "null" }))),
    defaultFilters());
});

test("a failed write is swallowed", () => {
  assert.doesNotThrow(() =>
    writeFilters(fakeHost(fakeStorage({}, true)), defaultFilters()));
});

test("blocked storage yields no value rather than throwing out of the accessor", () => {
  // app.js reads the favourites and the colour theme through these while its
  // module body is still executing. A browser with site data blocked throws
  // from getItem itself, before there is anything to parse, so a guard around
  // the parse alone still leaves no listeners bound and a blank page.
  assert.equal(readStored(fakeHost(fakeStorage({}, true)), "lamp.favorites"), null);
  assert.equal(readStored(fakeHost(fakeStorage({}, true)), "lamp.theme"), null);
  assert.doesNotThrow(() =>
    writeStored(fakeHost(fakeStorage({}, true)), "lamp.theme", "dark"));
  assert.doesNotThrow(() =>
    writeStored(fakeHost(fakeStorage({}, true)), "lamp.favorites", "[\"1\"]"));
});

test("the shared accessors round-trip a value and report a missing key as null", () => {
  const host = fakeHost(fakeStorage());
  writeStored(host, "lamp.theme", "dark");
  assert.equal(readStored(host, "lamp.theme"), "dark");
  assert.equal(readStored(host, "lamp.favorites"), null);
});

test("a throwing localStorage getter is guarded, not just a throwing getItem", () => {
  // The failure the guards exist for. Site data blocked for the origin makes
  // Window.localStorage itself raise, so the Storage never comes into being -
  // and a helper taking the Storage as its argument is handed that exception
  // at the call site, before its own try begins. Every one of these reaches
  // for the Storage inside its own try, which is the only place it is caught.
  const host = blockedHost();
  assert.throws(() => host.localStorage, { name: "SecurityError" },
    "the fixture must throw from the getter, else this proves nothing");

  // Control: the same helpers, on the same shape of host, still reach a real
  // Storage. Without this, "yields null" would also describe a helper that
  // never reached through the host at all, and the blocked case below would
  // pass for the wrong reason.
  const working = fakeHost(fakeStorage({ "lamp.theme": "dark" }));
  assert.equal(readStored(working, "lamp.theme"), "dark");
  assert.equal(readFilters(fakeHost(fakeStorage(
    { [FILTER_STORAGE_KEY]: '{"search":"tee"}' }))).search, "tee");

  assert.equal(readStored(host, "lamp.favorites"), null);
  assert.equal(readStored(host, "lamp.theme"), null);
  assert.deepEqual(readFilters(host), defaultFilters());
  assert.doesNotThrow(() => writeStored(host, "lamp.theme", "dark"));
  assert.doesNotThrow(() => writeStored(host, "lamp.favorites", "[\"1\"]"));
  assert.doesNotThrow(() => writeFilters(host, { ...defaultFilters(), search: "tee" }));
});

test("a host carrying no storage at all reads as no value", () => {
  // An embedder or a sandboxed frame can simply not expose one. Absent is not
  // an error condition for the viewer: it is the same as blocked.
  for (const host of [{}, { localStorage: null }, { localStorage: undefined }]) {
    assert.equal(readStored(host, "lamp.theme"), null);
    assert.deepEqual(readFilters(host), defaultFilters());
    assert.doesNotThrow(() => writeStored(host, "lamp.theme", "dark"));
    assert.doesNotThrow(() => writeFilters(host, defaultFilters()));
  }
});

test("matchSearch supports multi-term whitespace-separated matching in any order", () => {
  const title = "nike tech fleece black zip hoodie 2026";
  for (const [query, expected] of [
    ["", true], ["   ", true], ["nike", true], ["NIKE HOODIE", true],
    ["hoodie black 2026", true], [" fleece\tzip\nnike ", true],
    ["adidas", false], ["nike adidas", false],
  ]) {
    const matches = matchSearch(query);
    assert.equal(typeof matches, "function");
    assert.equal(matches(title), expected, query);
  }
});

test("a prepared search can be reused across titles without retaining matches", () => {
  const matches = matchSearch(" NIKE hoodie ");
  assert.equal(typeof matches, "function");
  assert.deepEqual(
    ["nike hoodie", "nike shoes", "hoodie nike", "", undefined].map(matches),
    [true, false, true, false, false]);
  assert.equal(matchSearch("")(undefined), true);
});

function album(id, extra = {}) {
  return { id: String(id), seller: extra.seller || "a", title: extra.title || id,
    price: extra.price || 0, photo_count: extra.photo_count || 0,
    published_ms: extra.published_ms || 0, category: extra.category || "" };
}

test("newest-first sorts the whole catalog, not seller groups", () => {
  // Seller-shard order would keep all of seller a together (newer, then older)
  // before seller b. Newest-first must interleave by published_ms.
  const albums = [
    album(1, { seller: "a", published_ms: 300, price: 180 }),
    album(2, { seller: "a", published_ms: 100, price: 50 }),
    album(3, { seller: "b", published_ms: 200, price: 90 }),
  ];
  assert.deepEqual(sortAlbums(albums, "default").map((a) => a.id), ["1", "3", "2"]);
  assert.deepEqual(sortAlbums(albums, "price-asc").map((a) => a.id), ["2", "3", "1"]);
  assert.deepEqual(sortAlbums(albums, "price-desc").map((a) => a.id), ["1", "3", "2"]);
});

test("price-asc puts unpriced albums last; price-desc treats them as 0", () => {
  const albums = [
    album(1, { price: 180 }),
    album(2, { price: 0 }),
    album(3, { price: 50 }),
  ];
  assert.deepEqual(sortAlbums(albums, "price-asc").map((a) => a.id), ["3", "1", "2"]);
  assert.deepEqual(sortAlbums(albums, "price-desc").map((a) => a.id), ["1", "3", "2"]);
});

test("most-photos sort and newest-first id tie-break", () => {
  const albums = [
    album(10, { photo_count: 2, published_ms: 5 }),
    album(30, { photo_count: 9, published_ms: 5 }),
    album(20, { photo_count: 9, published_ms: 5 }),
  ];
  assert.deepEqual(sortAlbums(albums, "photos-desc").map((a) => a.id), ["30", "20", "10"]);
  assert.deepEqual(sortAlbums(albums, "default").map((a) => a.id), ["30", "20", "10"]);
});

test("formatPrice hides a missing price and prefixes yen", () => {
  assert.equal(formatPrice(0), "");
  assert.equal(formatPrice(undefined), "");
  assert.equal(formatPrice(180), "¥180");
});

test("slash focuses search only when not already in a field", () => {
  assert.equal(isSearchShortcut("/", "DIV"), true);
  assert.equal(isSearchShortcut("/", ""), true);
  assert.equal(isSearchShortcut("/", "INPUT"), false);
  assert.equal(isSearchShortcut("/", "SELECT"), false);
  assert.equal(isSearchShortcut("/", "TEXTAREA"), false);
  assert.equal(isSearchShortcut("Escape", "DIV"), false);
});

test("favourites export refuses to run while shards are still loading", () => {
  const plan = favouritesExportPlan({
    favoriteIds: ["1"], albums: [album(1)], loading: true,
  });
  assert.equal(plan.ready, false);
  assert.equal(plan.reason, "loading");
});

test("favourites export reports omitted stars from missing shards", () => {
  const empty = favouritesExportPlan({ favoriteIds: [], albums: [album(1)], loading: false });
  assert.equal(empty.reason, "empty");

  const ok = favouritesExportPlan({
    favoriteIds: ["1"], albums: [album(1, { price: 180 })], loading: false,
  });
  assert.equal(ok.reason, "ok");
  assert.equal(ok.albums[0].price, 180);

  const partial = favouritesExportPlan({
    favoriteIds: ["1", "2"], albums: [album(1)], loading: false,
  });
  assert.equal(partial.reason, "partial");
  assert.deepEqual(partial.omittedIds, ["2"]);
  assert.equal(partial.ready, true);

  const missing = favouritesExportPlan({
    favoriteIds: ["9"], albums: [album(1)], loading: false,
  });
  assert.equal(missing.reason, "missing");
  assert.equal(missing.ready, false);
});

test("new + favourites + hide-dups still count as three independent filters", () => {
  const filters = { ...defaultFilters(), newOnly: true, favsOnly: true, hideDups: true };
  assert.equal(anyFilterActive(filters), true);
  assert.deepEqual(serializeFilters(filters),
    { newOnly: true, favsOnly: true, hideDups: true });
});
