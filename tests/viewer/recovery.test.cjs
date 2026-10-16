"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");
const {admittedCatalogueShard, inheritableAliases} = require("../../viewer/discovery.js");
const {createUserStore, inheritDuplicateWork, importUserBackup} = require("../../viewer/user-store.js");
const record = id => ({id, seller: "one", title: "Specific product", thumb: "", links: [],
  album_url: `https://one.x.yupoo.com/albums/${id}`});

test("selected-record admission checks membership and uniqueness before ignoring historical records", () => {
  const good = record("100"), bad = {...record("200"), links: [
    {platform: "item", url: "https://weidi\u200can.com/?itemID=7"}]};
  const seller = {id: "one", album_ids: ["100"]};
  assert.deepEqual(admittedCatalogueShard({albums: [good, bad]}, seller), [good]);
  assert.equal(admittedCatalogueShard({albums: [good, bad]}, {...seller, album_ids: ["200"]}), null);
  assert.equal(admittedCatalogueShard({albums: [good, good]}, seller), null);
  assert.equal(admittedCatalogueShard({albums: [bad]}, seller), null);
  assert.equal(admittedCatalogueShard({albums: [good, bad]}, {id: "one"}), null);
  assert.equal(admittedCatalogueShard({albums: [good, {...bad, seller: "other"}]}, seller), null);
});

test("manifest aliases need a compatible selection contract and an admitted destination", () => {
  const index = {album_aliases: {100: "200"}};
  assert.deepEqual(inheritableAliases(index, [record("200")], {}, false), {});
  assert.deepEqual(inheritableAliases({...index, selection_version: 1}, [], {}, true), {});
  assert.deepEqual(inheritableAliases({...index, selection_version: 1}, [record("200")], {}, false), {100: "200"});
  assert.deepEqual(inheritableAliases(index, [record("200")], {100: "200"}, true), {100: "200"});
});

test("fresh copies are independent of shard order; explicit fields and backup contents survive", () => {
  for (const order of [["100", "150", "300"], ["300", "100", "150"], ["150", "300", "100"]]) {
    const data = new Map();
    const store = createUserStore({localStorage: {get length() {return data.size;},
      key: i => [...data.keys()][i] ?? null, getItem: k => data.get(k) ?? null,
      setItem: (k, v) => data.set(k, v)}});
    for (const id of ["100", "150"]) {
      store.set("note", id, id === "150" ? "Later older note" : "Earlier note");
      store.set("favorite", id, true); store.set("tags", id, [id]); store.set("collection", id, id);
    }
    store.set("favorite", "300", false); store.set("tags", "300", []); store.set("collection", "300", "");
    store.set("review", "one", 1); store.set("dismiss", "100:150", true);
    for (let n = 1; n <= order.length; n++) {
      const loaded = order.slice(0, n);
      const newest = loaded.includes("300") ? "300" : loaded.includes("150") ? "150" : "100";
      const derived = Object.fromEntries(loaded.filter(id => id !== newest).map(id => [id, newest]));
      const aliases = inheritableAliases({}, [record(newest)], derived, n === order.length);
      inheritDuplicateWork(store, aliases);
      if (n < order.length) assert.equal(store.get("note", "300"), undefined);
    }
    assert.equal(store.get("note", "300"), "Later older note");
    assert.equal(store.get("favorite", "300"), false); assert.deepEqual(store.get("tags", "300"), []);
    assert.equal(store.get("collection", "300"), ""); assert.equal(store.get("note", "100"), "Earlier note");
    assert.equal(store.get("review", "one"), 1); assert.equal(store.get("dismiss", "300:150"), undefined);
    const backup = {format: "diogeneslamp", version: 1, entries: store.load()};
    assert.equal(importUserBackup(store, JSON.parse(JSON.stringify(backup))), 0);
    assert.deepEqual(store.load(), backup.entries);
  }
});
