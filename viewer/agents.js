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
  tmall: "tmall", "1688": "1688", ali_1688: "1688", alibaba: "1688" };
/** @type {Record<string, Marketplace>} */
const HOOBUY_CHANNELS = { "0": "1688", "1": "taobao", "2": "weidian" };
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
  if ([...values.values()].some(found => found.size > 1)) return null;
  /** @param {string} key */
  const param = key => values.get(key)?.values().next().value || "";
  for (const key of ["url", "productlink", "producturl", "link"]) {
    const inner = param(key);
    if (!inner) continue;
    // URLSearchParams decoded the one layer the agent applied; an agent
    // quoting another agent is one more unwrap, and nothing deeper.
    const item = AGENT_DEDUP.marketplaceItem(inner) ||
      AGENT_DEDUP.marketplaceItem(unwrapAgentLink(inner, depth + 1) || "");
    return item ? AGENT_DEDUP.marketplaceUrl(item) : null;
  }
  const ids = new Set(["id", "itemid", "goodsid"].map(param).filter(Boolean));
  const channels = new Set(["channel", "shop_type", "source"].map(param).filter(Boolean).map(v => v.toLowerCase()));
  if (ids.size > 1 || channels.size > 1) return null;
  const id = ids.values().next().value || "";
  const channel = channels.values().next().value || "";
  /** @param {Marketplace | undefined} platform @param {string} itemId */
  const item = (platform, itemId) => platform && /^[0-9]{1,24}$/.test(itemId)
    ? AGENT_DEDUP.marketplaceUrl({ platform, id: itemId }) : null;
  if (id && channel) return item(AGENT_PLATFORMS[channel], id);
  if (agent.id === "hoobuy") {
    const match = /^\/product\/([0-9])\/([0-9]+)\/?$/.exec(url.pathname);
    if (match) return item(HOOBUY_CHANNELS[match[1]], match[2]);
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
