"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");
const { AGENTS, ACTIVE_AGENTS, AGENT_HOSTS, unwrapAgentLink, agentLink } = require("../../viewer/agents.js");
const { marketplaceItem, marketplaceUrl } = require("../../viewer/dedup.js");

const IDS = { weidian: "7652831688", taobao: "560198996624", "1688": "691234567890" };
const WD = "https%3A%2F%2Fweidian.com%2Fitem.html%3FitemID%3D7652831688";
const TB = "https%3A%2F%2Fitem.taobao.com%2Fitem.htm%3Fid%3D560198996624";
const ALI = "https%3A%2F%2Fdetail.1688.com%2Foffer%2F691234567890.html";

// Read from rep.tools' public converter on 27 September 2026, minus its
// KakoBuy referral code.
const OBSERVED = {
  weidian: {
    kakobuy: `https://www.kakobuy.com/item/details?url=${WD}`,
    hoobuy: "https://hoobuy.com/product/2/7652831688",
    oopbuy: "https://oopbuy.com/goods/details?id=7652831688&channel=weidian",
    mulebuy: "https://mulebuy.com/product/?shop_type=weidian&id=7652831688",
    joyagoo: "https://joyagoo.com/product/?shop_type=weidian&id=7652831688",
    cssbuy: "https://www.cssbuy.com/item-micro-7652831688.html",
    sugargoo: `https://www.sugargoo.com/productDetail?productLink=${WD}`,
    superbuy: `https://www.superbuy.com/en/page/buy/?url=${WD}`,
    loongbuy: `https://www.loongbuy.com/product-details?url=${WD}`,
    pikobuy: `https://www.pikobuy.com/product/detail?productUrl=${WD}`,
  },
  taobao: {
    kakobuy: `https://www.kakobuy.com/item/details?url=${TB}`,
    hoobuy: "https://hoobuy.com/product/1/560198996624",
    oopbuy: "https://oopbuy.com/goods/details?id=560198996624&channel=taobao",
    mulebuy: "https://mulebuy.com/product/?shop_type=taobao&id=560198996624",
    joyagoo: "https://joyagoo.com/product/?shop_type=taobao&id=560198996624",
    cssbuy: "https://www.cssbuy.com/item-560198996624.html",
    sugargoo: `https://www.sugargoo.com/productDetail?productLink=${TB}`,
    superbuy: `https://www.superbuy.com/en/page/buy/?url=${TB}`,
    loongbuy: `https://www.loongbuy.com/product-details?url=${TB}`,
    pikobuy: `https://www.pikobuy.com/product/detail?productUrl=${TB}`,
  },
  "1688": {
    kakobuy: `https://www.kakobuy.com/item/details?url=${ALI}`,
    hoobuy: "https://hoobuy.com/product/0/691234567890",
    oopbuy: "https://oopbuy.com/goods/details?id=691234567890&channel=1688",
    mulebuy: "https://mulebuy.com/product/?shop_type=ali_1688&id=691234567890",
    joyagoo: "https://joyagoo.com/product/?shop_type=ali_1688&id=691234567890",
    cssbuy: "https://www.cssbuy.com/item-1688-691234567890.html",
    sugargoo: `https://www.sugargoo.com/productDetail?productLink=${ALI}`,
    superbuy: `https://www.superbuy.com/en/page/buy/?url=${ALI}`,
    loongbuy: `https://www.loongbuy.com/product-details?url=${ALI}`,
    pikobuy: `https://www.pikobuy.com/product/detail?productUrl=${ALI}`,
  },
};

test("every active agent rebuilds the observed link for each marketplace", () => {
  for (const [platform, expected] of Object.entries(OBSERVED)) {
    assert.deepEqual(Object.keys(expected).sort(), ACTIVE_AGENTS.map(a => a.id).sort(), platform);
    for (const [agentId, url] of Object.entries(expected)) {
      assert.equal(agentLink(agentId, { platform, id: IDS[platform] }), url, `${agentId} ${platform}`);
    }
  }
});

test("built links unwrap back to the item they were built from", () => {
  for (const agent of ACTIVE_AGENTS) {
    for (const [platform, id] of Object.entries(IDS)) {
      const item = { platform, id };
      assert.deepEqual(marketplaceItem(unwrapAgentLink(agentLink(agent.id, item))), item, `${agent.id} ${platform}`);
    }
    // Tmall rides the taobao channel on the agents that name channels, so
    // that trip is lossy by design and lands on the taobao item.
    const tmall = marketplaceItem(unwrapAgentLink(agentLink(agent.id, { platform: "tmall", id: "77" })));
    assert.ok(tmall && tmall.id === "77" && ["tmall", "taobao"].includes(tmall.platform), agent.id);
  }
});

test("retired agents still unwrap but never build, and stay out of the menu", () => {
  assert.equal(unwrapAgentLink("https://cnfans.com/product/?shop_type=weidian&id=7652831688"),
    "https://weidian.com/item.html?itemID=7652831688");
  assert.equal(unwrapAgentLink("https://www.pandabuy.com/product?url=" + TB), marketplaceUrl({ platform: "taobao", id: IDS.taobao }));
  assert.equal(agentLink("cnfans", { platform: "weidian", id: "1" }), "");
  assert.equal(agentLink("nobody", { platform: "weidian", id: "1" }), "");
  assert.equal(agentLink("hoobuy", null), "");
  assert.ok(AGENTS.filter(a => a.retired).every(a => !ACTIVE_AGENTS.includes(a)));
  assert.equal(new Set(AGENT_HOSTS).size, AGENTS.length);
  assert.ok(AGENTS.every(a => /^\d{4}-\d{2}-\d{2}$/.test(a.checked)));
});

test("unwrapping refuses ambiguity, other sites and deep nesting", () => {
  const evil = "https%3A%2F%2Fevil.example%2Fitem.html%3FitemID%3D1";
  for (const url of [
    `https://www.kakobuy.com/item/details?url=${evil}`,
    "https://oopbuy.com/goods/details?id=5&id=6&channel=weidian",
    "https://mulebuy.com/product/?shop_type=weidian&channel=taobao&id=5",
    "https://oopbuy.com/goods/details?id=5&channel=unknown",
    "https://hoobuy.com/product/9/123",
    "https://www.cssbuy.com/item-abc.html",
    "https://notkakobuy.com/item/details?url=" + WD,
    "https://weidian.com/item.html?itemID=7652831688",
    "not a url",
  ]) assert.equal(unwrapAgentLink(url), null, url);
  // One agent quoting another is one more unwrap; a third layer is not followed.
  const nested = "https://www.kakobuy.com/item/details?url=" + encodeURIComponent("https://hoobuy.com/product/2/7652831688");
  assert.equal(unwrapAgentLink(nested), "https://weidian.com/item.html?itemID=7652831688");
  const deeper = "https://www.kakobuy.com/item/details?url=" + encodeURIComponent(nested);
  assert.equal(unwrapAgentLink(deeper), null);
});
