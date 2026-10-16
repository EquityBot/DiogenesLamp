"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");
const {readFileSync} = require("node:fs");
const {join} = require("node:path");
const {dupKey, registerTitleKeys, newestListings} = require("../../viewer/dedup.js");
const {markNew} = require("../../viewer/catalogue.js");
const {kindOf} = require("../../viewer/kinds.js");
const {prepareAlbumSearch, admittedCatalogueShard, inheritableAliases} = require("../../viewer/discovery.js");
const {publishedMs} = require("../../viewer/age.js");
const {createViewState, selectCatalogue} = require("../../viewer/view-state.js");
const {defaultFilters} = require("../../viewer/filters.js");

const source = readFileSync(join(__dirname, "../../viewer/app.js"), "utf8");
function lift(name) {
  const found = source.match(new RegExp("\\n(?:async )?function " + name + "\\([\\s\\S]*?\\n\\}\\n"));
  assert.ok(found, "app.js no longer declares " + name);
  return found[0];
}

test("newest selection precedes seller and favourite filters in a legacy catalogue", () => {
  const old = {id: "100", seller: "one", title: "Old posting", title_lc: "old posting",
    date_published: "2026-01-01", links: [{url: "https://weidian.com/item.html?itemID=7"}]};
  const latest = {...old, id: "200", seller: "two", date_published: "2026-02-01"};
  for (const a of [old, latest]) { a.dup_key = dupKey(a); a.published_ms = publishedMs(a); }
  const state = {index: {sellers: [{id: "one"}, {id: "two"}]},
    loadedSellers: 2, failedShards: [],
    shardAlbums: new Map([["one", [old]], ["two", [latest]]])};
  const rebuild = new Function("state", "newestListings", "inheritableAliases", lift("rebuildAlbums") + "\nreturn rebuildAlbums;")(state, newestListings, inheritableAliases);
  rebuild();
  assert.deepEqual(state.albums, [latest]);
  assert.deepEqual(state.albumAliases, {100: "200"});
  for (const filters of [{seller: "one"}, {favsOnly: true}]) {
    const view = createViewState({...defaultFilters(), ...filters});
    assert.deepEqual(selectCatalogue(view, {albums: state.albums,
      favorites: new Set(["100"]), bannedKeys: new Set()}).filtered, []);
  }
});

test("indexed shards publish only selected records and report missing selections", async () => {
  const sellers = [{id: "one", shard: "catalog/one.json", album_ids: ["200"]},
    {id: "missing", shard: "catalog/missing.json", album_ids: ["300"]}];
  const record = id => ({id, seller: "one", title: "Blue shirt", links: [], thumb: "",
    album_url: `https://one.x.yupoo.com/albums/${id}`});
  const state = {pendingShards: [...sellers], generatedAt: "2026-10-01",
    titleKeyCounts: new Map(), bannedKeys: new Set(), categories: new Set(),
    shardAlbums: new Map(), failedShards: [], loadedSellers: 0};
  const prepared = [];
  const worker = new Function("state", "els", "fetch", "admittedCatalogueShard", "dupKey", "kindOf",
    "prepareAlbumSearch", "publishedMs", "markNew", "registerTitleKeys", "scheduleRefresh", "console",
    lift("nextShard") + lift("shardWorker") + "\nreturn shardWorker;"
  )(state, {seller: {value: ""}}, async url => ({ok: true,
    json: async () => ({albums: url.includes("missing") ? [] : [record("100"), record("200")]})}),
    admittedCatalogueShard, dupKey, kindOf, a => {prepared.push(a.id); prepareAlbumSearch(a);},
    publishedMs, markNew, registerTitleKeys, () => {}, {warn() {}});
  await worker();
  assert.deepEqual(prepared, ["200"]);
  assert.deepEqual(state.shardAlbums.get("one").map(a => a.id), ["200"]);
  assert.equal(state.shardAlbums.has("missing"), false);
  assert.deepEqual(state.failedShards, ["missing"]);
});

test("rebuilding a large seller keeps index order without an argument limit", () => {
  const first = Array.from({length: 200000}, (_, index) => ({id: String(index)}));
  const last = [{id: "last"}];
  const state = {albums: [{id: "old"}], index: {sellers: [
    {id: "first"}, {id: "missing"}, {id: "last"},
  ]}, shardAlbums: new Map([["last", last], ["first", first]])};
  const rebuild = new Function("state", "newestListings", "inheritableAliases", lift("rebuildAlbums") + "\nreturn rebuildAlbums;")(state, newestListings, inheritableAliases);
  rebuild();
  assert.equal(state.albums.length, 200001);
  assert.deepEqual(state.albums.slice(0, first.length), first);
  assert.equal(state.albums[0], first[0]);
  assert.equal(state.albums[first.length - 1], first.at(-1));
  assert.equal(state.albums.at(-1), last[0]);
  assert.equal(first.length, 200000);
  assert.deepEqual(last, [{id: "last"}]);
  state.shardAlbums.delete("first");
  rebuild();
  assert.deepEqual(state.albums, last);
});

test("large shards yield to input before publishing, then honour seller priority", async () => {
  const sellers = ["first", "second", "third"].map(id => ({id, shard: `catalog/${id}.json`}));
  const albums = seller => Array.from({length: seller === "first" ? 1200 : 1}, (_, index) => ({
    id: String(index + 1), seller, title: `blue shirt ${index + 1}`, category: "shirts",
    links: [], album_url: `https://${seller}.x.yupoo.com/albums/${index + 1}`, thumb: "",
    date_published: "2026-10-01", first_seen: "2026-10-01T00:00:00Z",
  }));
  const shards = new Map(sellers.map(seller => [seller.id, {albums: albums(seller.id)}]));
  const state = {pendingShards: [...sellers], generatedAt: "2026-10-01T00:00:00Z",
    titleKeyCounts: new Map(), bannedKeys: new Set(), categories: new Set(),
    shardAlbums: new Map(), failedShards: [], loadedSellers: 0};
  const els = {seller: {value: ""}};
  const requests = [];
  let prepared = 0, checkpoint, publishedAtInput, refreshes = 0;
  const worker = new Function("state", "els", "fetch", "admittedCatalogueShard", "dupKey", "kindOf",
    "prepareAlbumSearch", "publishedMs", "markNew", "registerTitleKeys", "scheduleRefresh",
    lift("nextShard") + lift("shardWorker") + "\nreturn shardWorker;"
  )(state, els, async url => {
    const id = /catalog\/(.+)\.json$/.exec(url)[1];
    requests.push(id);
    return {ok: true, json: async () => shards.get(id)};
  }, admittedCatalogueShard, dupKey, kindOf, album => {
    prepared += 1;
    prepareAlbumSearch(album);
  }, publishedMs, markNew, registerTitleKeys, () => { refreshes += 1; });
  const input = new Promise(resolve => setTimeout(() => {
    checkpoint = prepared;
    publishedAtInput = state.shardAlbums.has("first");
    els.seller.value = "third";
    resolve();
  }, 0));
  await worker();
  await input;
  assert.equal(publishedAtInput, false);
  assert.ok(checkpoint > 0 && checkpoint < 1200, `input ran after ${checkpoint} albums`);
  assert.deepEqual(requests, ["first", "third", "second"]);
  assert.equal(prepared, 1202);
  assert.equal(state.loadedSellers, 3);
  assert.equal(refreshes, 3);
  assert.deepEqual(state.failedShards, []);
  for (const seller of sellers) {
    assert.equal(state.shardAlbums.get(seller.id), shards.get(seller.id).albums);
    assert.ok(state.shardAlbums.get(seller.id).every(album =>
      album.search_keys && album.published_ms && album.kind));
  }
});
