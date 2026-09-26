"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");
const {prepareAlbumSearch, compileCatalogueSearch, catalogueCsv, similarCovers, validCatalogueShard} = require("../../viewer/discovery.js");
const {filtersToURL, filtersFromURL} = require("../../viewer/filters.js");
const {createViewState, selectCatalogue} = require("../../viewer/view-state.js");

const album = {id: "123", title: "ＡＢＣ cotton shirt", seller: "example", category: "shirts",
  first_seen: "2026-09-01", updated_at: "2026-09-02", price: 180, published_ms: 1,
  links: [{platform: "weidian", url: "https://weidian.com/item.html?itemID=456"}]};
prepareAlbumSearch(album);

test("IDs and links match exact items without substring false positives", () => {
  for (const q of ["123", "456", "https://example.x.yupoo.com/albums/123?uid=1",
    "https://weidian.com/item.html?itemID=456&utm_source=test", "abc cotton", "example shirt"]) {
    assert.equal(compileCatalogueSearch(q).matches(album), true, q);
  }
  for (const q of ["12", "45", "https://weidian.com/item.html?itemID=45", "https://unknown.test/123"]) {
    assert.equal(compileCatalogueSearch(q).matches(album), false, q);
  }
});
test("price bounds include endpoints, unknowns are explicit, inverted bounds show nothing", () => {
  const catalogue = {albums: [album, {...album, id: "124", price: 0}], favorites: new Set(), bannedKeys: new Set()};
  const select = filters => selectCatalogue(createViewState(filters), catalogue).filtered.map(a => a.id);
  assert.deepEqual(select({priceMin: "180", priceMax: "180"}), ["124", "123"]);
  assert.deepEqual(select({priceMin: "180", includeUnknown: false}), ["123"]);
  assert.deepEqual(select({priceMin: "181", priceMax: "180"}), []);
});
test("URL state round-trips all filters without importing arbitrary state", () => {
  const filters = createViewState({search: "中文 & shirt", priceMin: "80", includeUnknown: false,
    changed: true, collection: "Compare", tag: "gift"}).filters;
  assert.deepEqual(filtersFromURL(filtersToURL(filters)), filters);
  assert.equal(filtersFromURL("?q=anything"), null);
  assert.equal(filtersFromURL("?view=1&priceMin=-10").priceMin, "");
});
test("review cut-offs compare recorded instants, gone changes remain inspectable", () => {
  const changed = {...album, gone_since: "2026-09-03", history: [{at: "2026-09-03"}]};
  const catalogue = {albums: [changed], reviews: new Map([["example", Date.parse("2026-09-02")]]),
    favorites: new Set(), bannedKeys: new Set()};
  assert.equal(selectCatalogue(createViewState({changed: true}), catalogue).filtered.length, 1);
  assert.equal(selectCatalogue(createViewState({unreviewed: true}), catalogue).filtered.length, 0);
});
test("CSV quotes newlines and neutralises spreadsheet formulas, with partial scope", () => {
  const csv = catalogueCsv([{...album, title: '=HYPERLINK("bad")\nnext'}], "partial: seller unavailable");
  assert.ok(csv.startsWith("\uFEFF"));
  assert.ok(csv.includes('"\'=HYPERLINK(""bad"")\nnext"'));
  assert.ok(csv.includes("partial: seller unavailable"));
});

test("visual suggestions stay bounded, reject uniform covers and honour dismissals", () => {
  const source = {id:"1", cover_hash:"a5a5a5a5a5a5a5a5"};
  const same = {id:"2", cover_hash:source.cover_hash};
  const near = {id:"3", cover_hash:"a5a5a5a5a5a5a5a4"};
  const different = {id:"4", cover_hash:"5a5a5a5a5a5a5a5a"};
  assert.deepEqual(similarCovers([source, same, near, different], source).map(m=>[m.album.id,m.distance]), [["2",0],["3",1]]);
  assert.deepEqual(similarCovers([same, near],source,new Set(["2"])).map(m=>m.album.id),["3"]);
  assert.equal(similarCovers([same, near],source,new Set(),1).length,1);
  assert.deepEqual(similarCovers([{id:"5",cover_hash:"0000000000000000"}],{id:"6",cover_hash:"0000000000000000"}),[]);
});

test("malformed shards fail instead of accepting executable URLs or cover paths", () => {
  const record = {...album, thumb:"thumbs/example/123.jpg", album_url:"https://example.x.yupoo.com/albums/123"};
  assert.equal(validCatalogueShard({albums:[record]},"example"),true);
  for (const patch of [{thumb:"../other.jpg"},{album_url:"javascript:alert(1)"},{links:null},{id:123},{cover_hash:123}]) {
    assert.equal(validCatalogueShard({albums:[{...record,...patch}]},"example"),false);
  }
  assert.equal(validCatalogueShard({schema_version:2,albums:[record]},"example"),false);
  assert.equal(validCatalogueShard({albums:[record,record]},"example"),false);
});
