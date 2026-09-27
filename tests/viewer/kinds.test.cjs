"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");
const { KINDS, KIND_IDS, UNSORTED, kindOf } = require("../../viewer/kinds.js");

const CASES = [
  ["Nike Dunk Low Panda", "shoes"], ["Jordan 4 Black Cat", "shoes"], ["Yeezy Slide Onyx", "shoes"],
  ["Balenciaga Track LED", UNSORTED], ["Moncler Maya down jacket", "jackets"], ["Stone Island hoodie", "hoodies"],
  ["Essentials hoodie and sweatpants set", "sets"], ["Louis Vuitton belt", "accessories"],
  ["Watch strap replacement", "accessories"], ["Shoelaces 3 pairs", "accessories"], ["Rolex Submariner watch", "watches"],
  ["Cuban chain necklace", "jewellery"], ["Jewellery storage box", "accessories"], ["Short-sleeve shirt", "tees"],
  ["Nike shorts", "shorts"], ["Denim jeans", "pants"], ["Goyard tote bag", "bags"], ["Top quality version", UNSORTED],
  ["短袖T恤", "tees"], ["羽绒服", "jackets"], ["运动鞋", "shoes"], ["表带", "accessories"], ["首饰盒", "accessories"],
  ["", UNSORTED], [null, UNSORTED], ["https://weidian.com/shoes-shop", UNSORTED],
  ["ＡＦ１ white", "shoes"],
];

test("kindOf reads a garment kind from the title and abstains otherwise", () => {
  for (const [title, kind] of CASES) assert.equal(kindOf(title), kind, String(title));
});

test("the table is well formed: unique ids, lowercase tokens, unsorted last", () => {
  assert.equal(new Set(KIND_IDS).size, KIND_IDS.length);
  assert.equal(KIND_IDS.at(-1), UNSORTED);
  for (const kind of KINDS) {
    assert.ok(kind.latin.length && kind.label, kind.id);
    assert.ok(kind.latin.every(token => token === token.toLowerCase()), kind.id);
  }
  for (const [title] of CASES) assert.ok(KIND_IDS.includes(kindOf(title)));
});
