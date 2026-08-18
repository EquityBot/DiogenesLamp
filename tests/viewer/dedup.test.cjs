"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");
const path = require("node:path");

const { normTitle, titleWeight, itemKeys, dupKey, better, bannedTitleKeys,
        registerTitleKeys, collapseDups } =
  require(path.join(__dirname, "..", "..", "viewer", "dedup.js"));

function album(overrides) {
  return {
    id: "1", seller: "s", title: "", links: [],
    updated_at: "2026-08-17T21:00:00+10:00", dup_key: null,
    ...overrides,
  };
}

test("normTitle folds fullwidth, case, punctuation; keeps CJK", () => {
  assert.equal(normTitle("￥450【标准】Model One 样品"), "450标准modelone样品");
  assert.equal(normTitle("  Item-XY!! "), "itemxy");
  assert.equal(normTitle("Ｉｔｅｍ ＸＹ"), "itemxy");
  assert.equal(normTitle(""), "");
  assert.equal(normTitle(null), "");
});

test("itemKeys extracts item ids, ignores shop links", () => {
  assert.deepEqual(
    itemKeys([{ platform: "weidian", url: "https://weidian.com/item.html?itemID=783" }]),
    new Set(["weidian:783"]));
  assert.deepEqual(
    itemKeys([{ platform: "taobao", url: "https://item.taobao.com/item.htm?id=42" }]),
    new Set(["taobao:42"]));
  // shop link (userid only) and bare homepage carry no item identity
  assert.deepEqual(
    itemKeys([{ platform: "weidian", url: "https://weidian.com/?userid=99" },
              { platform: "taobao", url: "https://shop.taobao.com/" }]),
    new Set());
  // shortlinks identify items by path token
  assert.deepEqual(
    itemKeys([{ platform: "other", url: "https://m.tb.cn/h.ABC123" }]),
    new Set(["url:m.tb.cn/h.ABC123"]));
});

test("itemKeys survives fragments and malformed link entries", () => {
  // a stray '#detail' swallowed from album text must not split the key
  assert.deepEqual(
    itemKeys([{ platform: "taobao", url: "https://item.taobao.com/item.htm?id=695#detail" }]),
    new Set(["taobao:695"]));
  // missing/null/non-string urls and unparseable urls are skipped, never thrown
  assert.deepEqual(
    itemKeys([{ platform: "weidian" }, null, { platform: "taobao", url: 42 },
              { platform: "taobao", url: "http://" },
              { platform: "weidian", url: "https://weidian.com/item.html?itemID=7" }]),
    new Set(["weidian:7"]));
});

test("dupKey: single item link wins over title", () => {
  const a = album({
    title: "totally different title",
    links: [{ platform: "weidian", url: "https://weidian.com/item.html?itemID=7" }],
  });
  const b = album({
    title: "another name entirely",
    links: [{ platform: "weidian", url: "https://weidian.com/item.html?itemID=7" }],
  });
  assert.equal(dupKey(a), "i:weidian:7");
  assert.equal(dupKey(a), dupKey(b));
});

test("dupKey: multi-item catalogs and link-less albums fall back to title", () => {
  const catalog = album({
    title: "Summer Batch 12",
    links: [
      { platform: "weidian", url: "https://weidian.com/item.html?itemID=1" },
      { platform: "weidian", url: "https://weidian.com/item.html?itemID=2" },
    ],
  });
  assert.equal(dupKey(catalog), "t:s:summerbatch12");
  assert.equal(dupKey(album({ title: "Summer Batch 12" })), "t:s:summerbatch12");
  assert.equal(dupKey(album({ title: "" })), null);
  assert.equal(dupKey(album({ title: "!!!" })), null);
});

test("dupKey: title keys embed the seller, so titles never match cross-seller", () => {
  const a = album({ seller: "a", title: "Long Product Name ss26 brown" });
  const b = album({ seller: "b", title: "Long Product Name ss26 brown" });
  assert.notEqual(dupKey(a), dupKey(b));
  // item links still match across sellers
  const link = [{ platform: "weidian", url: "https://weidian.com/item.html?itemID=7" }];
  assert.equal(dupKey(album({ seller: "a", links: link })),
               dupKey(album({ seller: "b", links: link })));
});

test("dupKey: generic short titles never group; CJK weighs double", () => {
  // dozens of distinct products can share a title like "hat" - must not group
  assert.equal(dupKey(album({ title: "hat" })), null);
  assert.equal(dupKey(album({ title: "Item XY" })), null);
  assert.equal(titleWeight("hat"), 3);
  assert.equal(titleWeight(normTitle("样品4.0标准款")), 12);
  assert.equal(dupKey(album({ title: "样品4.0标准款" })), "t:s:样品40标准款");
});

test("better: more links, then newer updated_at, else keep incumbent", () => {
  const few = album({ links: [] });
  const many = album({
    links: [{ platform: "weidian", url: "https://weidian.com/item.html?itemID=1" }],
  });
  assert.equal(better(many, few), true);
  assert.equal(better(few, many), false);
  const older = album({ updated_at: "2026-08-01T00:00:00+10:00" });
  const newer = album({ updated_at: "2026-08-17T00:00:00+10:00" });
  assert.equal(better(newer, older), true);
  assert.equal(better(older, newer), false);
  assert.equal(better(album({}), album({})), false);
});

test("title groups: same-seller re-posts collapse, category labels never do", () => {
  const key = "t:a:longproductnamess26brown";
  // a seller posting the same title twice = re-post -> collapses
  const repost = [
    album({ id: "1", seller: "a", dup_key: key }),
    album({ id: "2", seller: "a", dup_key: key }),
  ];
  const { out, counts } = collapseDups(repost);
  assert.equal(out.length, 1);
  assert.equal(counts.get(key), 2);
  // 3+ same-title albums from one seller = category label -> banned
  const catalogish = [
    album({ id: "1", seller: "a", dup_key: key }),
    album({ id: "2", seller: "a", dup_key: key }),
    album({ id: "3", seller: "a", dup_key: key }),
  ];
  assert.deepEqual(bannedTitleKeys(catalogish), new Set([key]));
  assert.equal(collapseDups(catalogish).out.length, 3);
  // item-link keys group across sellers and ignore every title rule
  const itemDups = [
    album({ id: "1", seller: "a", dup_key: "i:weidian:7" }),
    album({ id: "2", seller: "b", dup_key: "i:weidian:7" }),
    album({ id: "3", seller: "a", dup_key: "i:weidian:7" }),
  ];
  assert.equal(collapseDups(itemDups).out.length, 1);
});

test("registerTitleKeys folds batches to the same bans as a full recompute", () => {
  const key = "t:a:autumnjacketcollectionvol1";
  const corpus = [
    album({ id: "1", seller: "a", dup_key: key }),
    album({ id: "2", seller: "a", dup_key: key }),
    album({ id: "3", seller: "a", dup_key: key }),
    album({ id: "4", seller: "b", dup_key: "t:b:someotherspecificproduct" }),
    album({ id: "5", seller: "b", dup_key: null }),
    album({ id: "6", seller: "b", dup_key: "i:weidian:7" }),
  ];
  const counts = new Map();
  const banned = new Set();
  // stream in two uneven batches, as concurrent shard loads would
  registerTitleKeys(corpus.slice(0, 2), counts, banned);
  assert.equal(banned.size, 0);
  registerTitleKeys(corpus.slice(2), counts, banned);
  assert.deepEqual(banned, bannedTitleKeys(corpus));
  assert.equal(counts.get(key), 3);
});

test("collapseDups honors a corpus-global banned set over a filtered subset", () => {
  const key = "t:a:autumnjacketcollectionvol1";
  const corpus = [
    album({ id: "1", seller: "a", dup_key: key }),
    album({ id: "2", seller: "a", dup_key: key }),
    album({ id: "3", seller: "a", dup_key: key }),
  ];
  const banned = bannedTitleKeys(corpus);
  // a filtered view (e.g. favorites) showing only 2 of the 3: without the
  // global set the pair would collapse and hide a distinct product
  const subset = corpus.slice(0, 2);
  assert.equal(collapseDups(subset, banned).out.length, 2);
  // the local default (subset-scoped) would wrongly collapse - the app
  // must always pass the corpus-global set
  assert.equal(collapseDups(subset).out.length, 1);
});

test("collapseDups keeps first position, swaps in better member, counts", () => {
  const k = "i:weidian:7";
  const a = album({ id: "a", dup_key: k, links: [] });
  const b = album({ id: "b", dup_key: k,
    links: [{ platform: "weidian", url: "https://weidian.com/item.html?itemID=7" }] });
  const solo = album({ id: "solo", dup_key: "t:other" });
  const nokey = album({ id: "nokey", dup_key: null });
  const { out, counts } = collapseDups([a, solo, b, nokey]);
  assert.deepEqual(out.map((x) => x.id), ["b", "solo", "nokey"]);
  assert.equal(counts.get(k), 2);
  assert.equal(counts.get("t:other"), 1);
});
