"use strict";

// Shopping agents: the hosts whose product links the scraper unwraps
// (src/lamp/links.py keeps the same list; a parity test holds them equal)
// and the URL templates that rebuild a link for an agent still trading.
// Every template is built from the bare marketplace item, never copied
// from a pasted link, so no referral code survives. Formats were read from
// the public link converters on 27 September 2026; `checked` records that.
// An agent that shut down keeps its parse rules and a `retired` date, so
// old links still resolve while the agent leaves the preference menu.

/** @typedef {"taobao" | "tmall" | "weidian" | "1688"} Marketplace */
/** @typedef {{platform: Marketplace, id: string}} MarketplaceItem */
/** @typedef {{id: string, name: string, host: string, checked: string, retired?: string,
 *   build?: (item: MarketplaceItem) => string}} Agent */

const AGENT_DEDUP = typeof module !== "undefined" ? require("./dedup.js") : { marketplaceItem, marketplaceUrl };

/** @param {MarketplaceItem} item */
const wrapped = item => encodeURIComponent(AGENT_DEDUP.marketplaceUrl(item));

// Agents that name the marketplace by a channel code have no code for tmall:
// their taobao channel opens the item, but a link built that way unwraps to
// a taobao identity and would miss the tmall album it came from. Such
// agents offer no tmall link; the marketplace link itself stays available.
/**
 * @param {Partial<Record<Marketplace, string | number>>} channels
 * @param {(channel: string | number, id: string) => string} template
 * @returns {(item: MarketplaceItem) => string}
 */
const channelled = (channels, template) => item => {
  const channel = channels[item.platform];
  return channel === undefined ? "" : template(channel, item.id);
};

/** @type {Agent[]} */
const AGENTS = [
  { id: "kakobuy", name: "KakoBuy", host: "kakobuy.com", checked: "2026-09-27",
    build: item => `https://www.kakobuy.com/item/details?url=${wrapped(item)}` },
  { id: "hoobuy", name: "Hoobuy", host: "hoobuy.com", checked: "2026-09-27",
    build: channelled({ "1688": 0, taobao: 1, weidian: 2 }, (channel, id) => `https://hoobuy.com/product/${channel}/${id}`) },
  { id: "oopbuy", name: "Oopbuy", host: "oopbuy.com", checked: "2026-09-27",
    build: channelled({ "1688": "1688", taobao: "taobao", weidian: "weidian" },
      (channel, id) => `https://oopbuy.com/goods/details?id=${id}&channel=${channel}`) },
  { id: "mulebuy", name: "Mulebuy", host: "mulebuy.com", checked: "2026-09-27",
    build: channelled({ "1688": "ali_1688", taobao: "taobao", weidian: "weidian" },
      (channel, id) => `https://mulebuy.com/product/?shop_type=${channel}&id=${id}`) },
  { id: "joyagoo", name: "Joyagoo", host: "joyagoo.com", checked: "2026-09-27",
    build: channelled({ "1688": "ali_1688", taobao: "taobao", weidian: "weidian" },
      (channel, id) => `https://joyagoo.com/product/?shop_type=${channel}&id=${id}`) },
  { id: "cssbuy", name: "CSSBuy", host: "cssbuy.com", checked: "2026-09-27",
    build: channelled({ "1688": "1688-", taobao: "", weidian: "micro-" },
      (prefix, id) => `https://www.cssbuy.com/item-${prefix}${id}.html`) },
  { id: "sugargoo", name: "Sugargoo", host: "sugargoo.com", checked: "2026-09-27",
    build: item => `https://www.sugargoo.com/productDetail?productLink=${wrapped(item)}` },
  { id: "superbuy", name: "Superbuy", host: "superbuy.com", checked: "2026-09-27",
    build: item => `https://www.superbuy.com/en/page/buy/?url=${wrapped(item)}` },
  { id: "loongbuy", name: "LoongBuy", host: "loongbuy.com", checked: "2026-09-27",
    build: item => `https://www.loongbuy.com/product-details?url=${wrapped(item)}` },
  { id: "pikobuy", name: "Pikobuy", host: "pikobuy.com", checked: "2026-09-27",
    build: item => `https://www.pikobuy.com/product/detail?productUrl=${wrapped(item)}` },
  { id: "itaobuy", name: "iTaobuy", host: "itaobuy.com", checked: "2026-09-27",
    build: item => `https://www.itaobuy.com/product-detail?url=${wrapped(item)}` },
  { id: "bbdbuy", name: "BBDbuy", host: "bbdbuy.com", checked: "2026-09-27",
    build: item => `https://bbdbuy.com/index/item/${item.platform === "1688" ? "index1688" : "index"}.html?tp=taobao&tid=&searchlang=en&url=${wrapped(item)}` },
  { id: "ootdbuy", name: "Ootdbuy", host: "ootdbuy.com", checked: "2026-09-27",
    build: channelled({ "1688": "1688", taobao: "TAOBAO", weidian: "weidian" },
      (channel, id) => `https://www.ootdbuy.com/goods/details?id=${id}&channel=${channel}`) },
  { id: "lovegobuy", name: "Lovegobuy", host: "lovegobuy.com", checked: "2026-09-27",
    build: channelled({ "1688": "ali_1688", taobao: "taobao", weidian: "weidian" },
      (channel, id) => `https://www.lovegobuy.com/product?id=${id}&shop_type=${channel}`) },
  { id: "cnshopper", name: "Cnshopper", host: "cnshopper.com", checked: "2026-09-27",
    build: channelled({ "1688": "1688", taobao: "taobao", weidian: "weidian" },
      (channel, id) => `https://cnshopper.com/goods/detail?keyword=${id}&platform=${channel}`) },
  { id: "alleurobuy", name: "Alleurobuy", host: "alleurobuy.com", checked: "2026-09-27",
    build: channelled({ "1688": "ali", taobao: "tao", weidian: "weidian" },
      (channel, id) => `https://alleurobuy.com/index/detail-public?id=${id}&type=${channel}`) },
  { id: "rizzitgo", name: "RizzitGO", host: "rizzitgo.com", checked: "2026-09-27",
    build: channelled({ "1688": 2, taobao: 1, weidian: 3 },
      (channel, id) => `https://rizzitgo.com/detail-page?goodsId=${id}&source=${channel}`) },
  { id: "vigorbuy", name: "Vigorbuy", host: "vigorbuy.com", checked: "2026-09-27",
    build: channelled({ "1688": 0, taobao: 1, weidian: 2 }, (channel, id) => `https://vigorbuy.com/product/${channel}/${id}`) },
  { id: "usfans", name: "USFans", host: "usfans.com", checked: "2026-09-27",
    build: channelled({ "1688": 1, taobao: 2, weidian: 3 }, (channel, id) => `https://usfans.com/product/${channel}/${id}`) },
  { id: "litbuy", name: "Litbuy", host: "litbuy.com", checked: "2026-09-27",
    build: channelled({ "1688": 0, taobao: 1, weidian: "weidian" }, (channel, id) => `https://www.litbuy.com/product/${channel}/${id}`) },
  { id: "hipobuy", name: "hipobuy", host: "hipobuy.com", checked: "2026-09-27",
    build: channelled({ "1688": 0, taobao: 1, weidian: "weidian" },
      (channel, id) => `https://hipobuy.com/product/${channel}/${id}${channel === 0 ? ".html" : ""}`) },
  { id: "gtbuy", name: "GTBuy", host: "gtbuy.com", checked: "2026-09-27",
    build: channelled({ "1688": 0, taobao: 1, weidian: "weidian" }, (channel, id) => `https://www.gtbuy.com/product/${channel}/${id}`) },
  { id: "cnfans", name: "CNFans", host: "cnfans.com", checked: "2026-09-27", retired: "2026-08" },
  { id: "allchinabuy", name: "AllChinaBuy", host: "allchinabuy.com", checked: "2026-09-27", retired: "2026-08" },
  { id: "pandabuy", name: "Pandabuy", host: "pandabuy.com", checked: "2026-09-27", retired: "2024-04" },
];

const AGENT_HOSTS = AGENTS.map(agent => agent.host);
const ACTIVE_AGENTS = AGENTS.filter(agent => !agent.retired && agent.build);

/** @param {string} hostname @param {string} host */
function hostWithin(hostname, host) {
  return hostname === host || hostname.endsWith("." + host);
}

/** @param {string} hostname */
function agentFor(hostname) {
  return AGENTS.find(agent => hostWithin(hostname, agent.host)) || null;
}

/** @type {Record<string, Marketplace>} */
const AGENT_PLATFORMS = { weidian: "weidian", wd: "weidian", taobao: "taobao", tb: "taobao",
  tmall: "tmall", "1688": "1688", ali_1688: "1688", alibaba: "1688", tao: "taobao", ali: "1688" };
/** @type {Record<string, Marketplace>} */
const HOOBUY_CHANNELS = { "0": "1688", "1": "taobao", "2": "weidian" };
/** @type {Record<string, Marketplace>} */
const USFANS_CHANNELS = { "1": "1688", "2": "taobao", "3": "weidian" };
/** @type {Record<string, Marketplace>} */
const NAMED_CHANNELS = { "0": "1688", "1": "taobao", weidian: "weidian" };
// Agents that name the marketplace in the path, /product/<channel>/<id>, and
// how each numbers or names its channels. links.py keeps the same maps.
/** @type {Record<string, Record<string, Marketplace>>} */
const PATH_CHANNELS = { hoobuy: HOOBUY_CHANNELS, vigorbuy: HOOBUY_CHANNELS, usfans: USFANS_CHANNELS,
  litbuy: NAMED_CHANNELS, hipobuy: NAMED_CHANNELS, gtbuy: NAMED_CHANNELS };
/** @type {Record<string, Marketplace>} */
const RIZZITGO_SOURCES = { "1": "taobao", "2": "1688", "3": "weidian" };
// Each agent names its item one way only: wrapped agents carry the whole
// marketplace URL in a parameter, query agents name the id and marketplace
// in the two parameters listed here, path agents are in PATH_CHANNELS and
// cssbuy spells the marketplace in its file name. links.py keeps the same.
const WRAPPED_AGENTS = new Set(["kakobuy", "superbuy", "sugargoo", "loongbuy", "pikobuy", "itaobuy", "bbdbuy",
  "allchinabuy", "pandabuy"]);
/** @type {Record<string, [string, string]>} */
const AGENT_QUERY = { oopbuy: ["id", "channel"], ootdbuy: ["id", "channel"], mulebuy: ["id", "shop_type"],
  joyagoo: ["id", "shop_type"], cnfans: ["id", "shop_type"], lovegobuy: ["id", "shop_type"],
  cnshopper: ["keyword", "platform"], alleurobuy: ["id", "type"], rizzitgo: ["goodsid", "source"] };
const WRAPPED_PARAMS = ["url", "productlink", "producturl", "link"];
const ID_PARAMS = ["id", "itemid", "goodsid", "keyword"];
const CHANNEL_PARAMS = ["channel", "shop_type", "source", "platform", "type"];
/** @type {Record<string, Marketplace>} */
const CSSBUY_PREFIXES = { "": "taobao", "micro-": "weidian", "1688-": "1688" };

/**
 * The marketplace item an agent product link points at, as its canonical
 * marketplace URL, or null when the link is not an agent link or carries
 * nothing usable. Mirrors links.py so a pasted agent link finds the same
 * album the scraper keyed.
 * @param {string} raw
 * @param {number} [depth]
 * @returns {string | null}
 */
function unwrapAgentLink(raw, depth = 0) {
  let url;
  try { url = new URL(raw); } catch { return null; }
  const agent = agentFor(url.hostname.toLowerCase());
  if (!agent || depth >= 2) return null;
  // A parameter repeated with different values names no single item:
  // refuse the ambiguity rather than trust whichever came first.
  /** @type {Map<string, Set<string>>} */
  const values = new Map();
  for (const [k, v] of url.searchParams) {
    if (!v) continue;
    const key = k.toLowerCase();
    values.set(key, (values.get(key) || new Set()).add(v));
  }
  // Only the parameters an agent's own format reads can make its link
  // ambiguous: a path agent names the item in the path whatever rides in
  // the query. A parameter repeated with different values, or two item ids
  // or marketplace hints on a query agent, names no single item.
  /** @param {string[]} keys */
  const ambiguous = keys => keys.some(key => (values.get(key)?.size || 0) > 1);
  /** @param {string} key */
  const param = key => values.get(key)?.values().next().value || "";
  if (WRAPPED_AGENTS.has(agent.id)) {
    if (ambiguous(WRAPPED_PARAMS)) return null;
    for (const key of WRAPPED_PARAMS) {
      const inner = param(key);
      if (!inner) continue;
      // URLSearchParams decoded the one layer the agent applied; an agent
      // quoting another agent is one more unwrap, and nothing deeper.
      const item = AGENT_DEDUP.marketplaceItem(inner) ||
        AGENT_DEDUP.marketplaceItem(unwrapAgentLink(inner, depth + 1) || "");
      return item ? AGENT_DEDUP.marketplaceUrl(item) : null;
    }
    return null;
  }
  /** @param {Marketplace | undefined} platform @param {string} itemId */
  const item = (platform, itemId) => platform && /^[0-9]{1,24}$/.test(itemId)
    ? AGENT_DEDUP.marketplaceUrl({ platform, id: itemId }) : null;
  const query = AGENT_QUERY[agent.id];
  if (query) {
    const ids = new Set(ID_PARAMS.map(param).filter(Boolean));
    const channels = new Set(CHANNEL_PARAMS.map(param).filter(Boolean).map(v => v.toLowerCase()));
    if (ambiguous([...ID_PARAMS, ...CHANNEL_PARAMS]) || ids.size > 1 || channels.size > 1) return null;
    const id = param(query[0]);
    const channel = param(query[1]).toLowerCase();
    if (!id || !channel) return null;
    return item((agent.id === "rizzitgo" ? RIZZITGO_SOURCES : AGENT_PLATFORMS)[channel], id);
  }
  const codes = PATH_CHANNELS[agent.id];
  if (codes) {
    const match = /^\/product\/([A-Za-z0-9]+)\/([0-9]+)(?:\.html)?\/?$/.exec(url.pathname);
    if (match) return item(codes[match[1].toLowerCase()], match[2]);
  }
  if (agent.id === "cssbuy") {
    const match = /^\/item-(micro-|1688-)?([0-9]+)\.html$/.exec(url.pathname);
    if (match) return item(CSSBUY_PREFIXES[match[1] || ""], match[2]);
  }
  return null;
}

/**
 * The chosen agent's link for a marketplace item, or "" when the agent is
 * unknown, retired or has no template for that marketplace.
 * @param {string} agentId
 * @param {MarketplaceItem | null | undefined} item
 */
function agentLink(agentId, item) {
  const agent = ACTIVE_AGENTS.find(candidate => candidate.id === agentId);
  if (!agent || !agent.build || !item || !(item.platform in AGENT_PLATFORMS)) return "";
  return agent.build(item);
}

if (typeof module !== "undefined") {
  module.exports = { AGENTS, AGENT_HOSTS, ACTIVE_AGENTS, agentFor, unwrapAgentLink, agentLink };
}
