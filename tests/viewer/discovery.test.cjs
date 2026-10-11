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

test("pasted agent and 1688 links find the album they name", () => {
  const { keyForUrl } = require("../../viewer/discovery.js");
  const wd = { ...album, id: "200", links: [{ platform: "weidian", url: "https://weidian.com/item.html?itemID=7652831688" }] };
  const ali = { ...album, id: "201", links: [{ platform: "1688", url: "https://detail.1688.com/offer/691234567890.html" }] };
  prepareAlbumSearch(wd); prepareAlbumSearch(ali);
  for (const q of ["https://hoobuy.com/product/2/7652831688",
    "https://www.kakobuy.com/item/details?url=https%3A%2F%2Fweidian.com%2Fitem.html%3FitemID%3D7652831688&affcode=x",
    "https://cnfans.com/product/?shop_type=weidian&id=7652831688"]) {
    assert.equal(compileCatalogueSearch(q).reason(wd), "Purchase link", q);
    assert.equal(compileCatalogueSearch(q).matches(ali), false, q);
  }
  assert.equal(compileCatalogueSearch("https://m.1688.com/offer/691234567890.html").reason(ali), "Purchase link");
  assert.equal(compileCatalogueSearch("691234567890").reason(ali), "Exact item ID");
  assert.equal(keyForUrl("https://weidian.com/?userid=5"), "");
  // A pasted link without its scheme, as album text carries them, still
  // keys on the item when its host is a known agent or marketplace.
  for (const q of ["cnshopper.com/goods/detail?keyword=7652831688&platform=weidian",
    "hoobuy.com/product/2/7652831688", "weidian.com/item.html?itemID=7652831688"]) {
    assert.equal(compileCatalogueSearch(q).reason(wd), "Purchase link", q);
  }
  assert.equal(compileCatalogueSearch("m.1688.com/offer/691234567890.html").reason(ali), "Purchase link");
  // An unknown host with a dot is a text search, not a failed link.
  const named = { ...album, id: "202", title: "see nike.com/air for the sample" };
  prepareAlbumSearch(named);
  assert.equal(compileCatalogueSearch("nike.com/air").reason(named), "Title");
});

test("descriptions are searched after titles and named as the reason", () => {
  const withText = { ...album, id: "300", title: "Plain hoodie", description: "Batch LJR, code HD-2201" };
  prepareAlbumSearch(withText);
  assert.equal(compileCatalogueSearch("hd-2201").reason(withText), "Description");
  assert.equal(compileCatalogueSearch("hoodie ljr").reason(withText), "Description");
  assert.equal(compileCatalogueSearch("hoodie").reason(withText), "Title");
  assert.equal(compileCatalogueSearch("hd-2201").matches(album), false);
});

test("CSV carries descriptions, kinds and every agent link for the chosen agent", () => {
  const two = { ...album, description: "Size chart in photos", kind: "tees", links: [
    { platform: "weidian", url: "https://weidian.com/item.html?itemID=1" },
    { platform: "taobao", url: "https://item.taobao.com/item.htm?id=2" }] };
  const csv = catalogueCsv([two], "complete", "hoobuy");
  assert.ok(csv.includes('"description","kind","agent_links"'));
  assert.ok(csv.includes('"https://hoobuy.com/product/2/1 https://hoobuy.com/product/1/2"'));
  assert.ok(csv.includes('"Size chart in photos","tees"'));
  assert.ok(catalogueCsv([two], "complete").endsWith(',"tees",""\r\n'), "no agent chosen leaves the column empty");
});

test("spreadsheet cross-reference reads quoted cells, semicolons and BOMs and keys only URLs", () => {
  const { sheetItemKeys, parseDelimited } = require("../../viewer/discovery.js");
  const text = "﻿name;price;link\r\n" +
    "\"Fur slides, black\";27.43;\"https://www.kakobuy.com/item/details?url=https%3A%2F%2Fweidian.com%2Fitem.html%3FitemID%3D7652831688\"\r\n" +
    "Runner;115;hoobuy.com/product/1/560198996624 and https://detail.1688.com/offer/691234567890.html.\r\n" +
    "No link;42;see notes 7652831688\n";
  const sheet = sheetItemKeys(text);
  assert.equal(sheet.rows, 4);
  assert.equal(sheet.urls, 3);
  assert.deepEqual([...sheet.keys].sort(), ["1688:691234567890", "taobao:560198996624", "weidian:7652831688"]);
  assert.deepEqual(parseDelimited('a,"b ""q"" c",d\n1,,3'), [["a", 'b "q" c', "d"], ["1", "", "3"]]);
  assert.throws(() => sheetItemKeys("a\nb\nc", 2));
});

test("spreadsheet candidates start at a URL boundary, so lookalike hosts yield nothing", () => {
  const { sheetItemKeys } = require("../../viewer/discovery.js");
  const sheet = sheetItemKeys("evil-taobao.com/item.htm?id=123,https://evil-taobao.com/item.htm?id=124," +
    "taobao.com.evil.example/item.htm?id=125,user@weidian.com/item.html?itemID=126,https://item.taobao.com/item.htm?id=127");
  assert.deepEqual([...sheet.keys], ["taobao:127"]);
});

test("spreadsheet URLs trim trailing punctuation and retain a following non-punctuation suffix", () => {
  const { sheetItemKeys } = require("../../viewer/discovery.js");
  const url = "https://weidian.com/item.html?itemID=123";
  const punctuation = ".".repeat(50000);
  const trimmed = sheetItemKeys(url + punctuation);
  assert.equal(trimmed.urls, 1);
  assert.deepEqual([...trimmed.keys], ["weidian:123"]);
  const retained = sheetItemKeys(url + punctuation + "x");
  assert.equal(retained.urls, 1);
  assert.deepEqual([...retained.keys], []);
});

test("the delimiter is read outside quotes, so quoted commas and newlines survive a semicolon sheet", () => {
  const { parseDelimited, matchSheet } = require("../../viewer/discovery.js");
  const rows = parseDelimited('name;"a,b,c\ncontinued";link\r\nx;y;https://weidian.com/item.html?itemID=1\r\n');
  assert.deepEqual(rows, [["name", "a,b,c\ncontinued", "link"], ["x", "y", "https://weidian.com/item.html?itemID=1"]]);
  // an album with two links in the sheet counts once, and both items count as found
  const two = { id: "9", search_keys: ["weidian:1", "taobao:2"] };
  const one = { id: "8", search_keys: ["taobao:2"] };
  const none = { id: "7", search_keys: ["weidian:5"] };
  const result = matchSheet([two, one, none], new Set(["weidian:1", "taobao:2", "1688:3"]));
  assert.deepEqual(result.matched.map(a => a.id), ["9", "8"]);
  assert.deepEqual([...result.found].sort(), ["taobao:2", "weidian:1"]);
});
