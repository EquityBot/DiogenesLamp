"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");
const {readFileSync} = require("node:fs");
const {join} = require("node:path");
const {dupKey, registerTitleKeys} = require("../../viewer/dedup.js");
const {markNew} = require("../../viewer/catalogue.js");
const {kindOf} = require("../../viewer/kinds.js");
const {prepareAlbumSearch, validCatalogueShard} = require("../../viewer/discovery.js");
const {publishedMs} = require("../../viewer/age.js");

const source = readFileSync(join(__dirname, "../../viewer/app.js"), "utf8");
function lift(name) {
  const found = source.match(new RegExp("\\n(?:async )?function " + name + "\\([\\s\\S]*?\\n\\}\\n"));
  assert.ok(found, "app.js no longer declares " + name);
  return found[0];
}

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
  const worker = new Function("state", "els", "fetch", "validCatalogueShard", "dupKey", "kindOf",
    "prepareAlbumSearch", "publishedMs", "markNew", "registerTitleKeys", "scheduleRefresh",
    lift("nextShard") + lift("shardWorker") + "\nreturn shardWorker;"
  )(state, els, async url => {
    const id = /catalog\/(.+)\.json$/.exec(url)[1];
    requests.push(id);
    return {ok: true, json: async () => shards.get(id)};
  }, validCatalogueShard, dupKey, kindOf, album => {
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
