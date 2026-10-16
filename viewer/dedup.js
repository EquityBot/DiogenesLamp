"use strict";

/**
 * @typedef {{url?: unknown, platform?: string} | null | undefined} PurchaseLink
 * @typedef {{links: PurchaseLink[], updated_at?: string | null, dup_key?: string | null}} DuplicateAlbum
 */

// Duplicate-posting detection. Pure functions only - no DOM access - so the
// same file runs in the browser (script tag before app.js) and under node
// for tests.

// Titles compare after NFKC folding (fullwidth yen/digits -> ASCII), case
// folding, and stripping everything that is not a letter or digit. CJK
// characters are letters and survive.
/**
 * @param {string | null | undefined} title
 */
function normTitle(title) {
  return (title || "")
    .normalize("NFKC")
    .toLowerCase()
    .replace(/[^\p{L}\p{N}]+/gu, "");
}

// Shortlinks identify an item by path token rather than an id. links.py
// keeps the same list; a parity test holds them equal.
const SHORTLINK_HOSTS = ["m.tb.cn", "k.youshop10.com"];

// The item page each marketplace serves for a bare item id, as links.py
// builds it when it unwraps an agent link.
/** @type {Record<import("./agents.js").Marketplace, string>} */
const ITEM_URLS = {
  taobao: "https://item.taobao.com/item.htm?id=",
  tmall: "https://detail.tmall.com/item.htm?id=",
  weidian: "https://weidian.com/item.html?itemID=",
  "1688": "https://detail.1688.com/offer/",
};

/** @param {string} hostname @param {string} host */
function hostIs(hostname, host) {
  return hostname === host || hostname.endsWith("." + host);
}

// The marketplace item a URL points at: taobao/tmall/weidian item pages
// carry ?id= / ?itemID=, 1688 offers carry the id in the path. Shop and
// homepage links (weidian ?userid=, bare domains) identify the seller, not
// an item, and yield null - otherwise every album that repeats the seller's
// shop link would collapse into one group. URL() does the parsing so stray
// fragments ("?id=695#detail" pasted from album text) and encoding never
// leak into the key, and a malformed string yields null instead of throwing.
/**
 * @param {unknown} raw
 * @returns {import("./agents.js").MarketplaceItem | null}
 */
function marketplaceItem(raw) {
  if (typeof raw !== "string") return null;
  let url;
  try { url = new URL(raw); } catch { return null; }
  const hostname = url.hostname.toLowerCase();
  /** @type {import("./agents.js").Marketplace | null} */
  const platform = hostIs(hostname, "taobao.com") ? "taobao" : hostIs(hostname, "tmall.com") ? "tmall"
    : hostIs(hostname, "weidian.com") ? "weidian" : hostIs(hostname, "1688.com") ? "1688" : null;
  if (!platform) return null;
  // Repeated, conflicting ids ("?id=1&id=2") name no single item: reject
  // the ambiguity rather than trust whichever came first.
  const ids = new Set();
  for (const [k, v] of url.searchParams) {
    const kl = k.toLowerCase();
    if (v && (kl === "id" || kl === "itemid")) ids.add(v);
  }
  if (platform === "1688") {
    const offer = /^\/offer\/([0-9]+)\.html$/.exec(url.pathname)?.[1];
    if (offer) ids.add(offer);
  }
  if (ids.size !== 1) return null;
  const id = ids.values().next().value;
  return /^[0-9]{1,24}$/.test(id) ? { platform, id } : null;
}

/** @param {import("./agents.js").MarketplaceItem} item */
function marketplaceUrl(item) {
  return ITEM_URLS[item.platform] + item.id + (item.platform === "1688" ? ".html" : "");
}

// One identity key per link that points at a single purchasable item. The
// platform comes from the validated host, never from the stored label, so
// a record cannot claim a weidian identity with a link elsewhere; tmall and
// taobao ids stay in separate namespaces as before. A link entry whose url
// is missing or malformed is skipped instead of throwing - one bad link
// must not cost the whole shard.
/**
 * @param {PurchaseLink[]} links
 * @returns {Set<string>}
 */
function itemKeys(links) {
  const keys = new Set();
  for (const link of links) {
    if (!link || typeof link.url !== "string") continue;
    const item = marketplaceItem(link.url);
    if (item) { keys.add(`${item.platform}:${item.id}`); continue; }
    let url;
    try { url = new URL(link.url); } catch { continue; }
    if (SHORTLINK_HOSTS.includes(url.hostname) && url.pathname.length > 1) {
      keys.add(`url:${url.hostname}${url.pathname}`);
    }
  }
  return keys;
}

// Generic one-word titles ("hat", "shoes") name a category, not a product -
// a live check found dozens of distinct products collapsing into one
// "hat" group.
// Only titles carrying enough signal may group. CJK characters pack roughly
// a word per character, so they weigh double.
const _TITLE_MIN_WEIGHT = 8;

/**
 * @param {string} normalized
 */
function titleWeight(normalized) {
  let weight = 0;
  for (const ch of normalized) {
    weight += /** @type {number} */ (ch.codePointAt(0)) > 0x2e7f ? 2 : 1;
  }
  return weight;
}

// Group key for an album, or null when the album has no usable identity.
// Exactly one item link -> that item is the identity (works across sellers
// even when titles differ). Zero or several item links (link-less albums,
// multi-item catalogues) -> fall back to the normalised title when it is
// specific enough. The seller is baked into title keys so titles can never
// group across sellers: live-corpus sampling showed every cross-seller
// title match was mirrored storefronts sharing category taxonomy - one
// title on 5 sellers, 5 catalogues - not reposts. Genuine reposts
// share purchase links, and the item-key path catches those.
/**
 * @param {{links?: PurchaseLink[], title?: string | null, seller: string}} album
 */
function dupKey(album) {
  const keys = itemKeys(album.links || []);
  if (keys.size === 1) return `i:${keys.values().next().value}`;
  const title = normTitle(album.title);
  if (titleWeight(title) < _TITLE_MIN_WEIGHT) return null;
  return `t:${album.seller}:${title}`;
}

// True when a should represent its group instead of b: more purchase links
// wins, then newer updated_at; otherwise keep the incumbent (stable order).
/**
 * @param {DuplicateAlbum} a
 * @param {DuplicateAlbum} b
 */
function better(a, b) {
  if (a.links.length !== b.links.length) return a.links.length > b.links.length;
  const ta = Date.parse(a.updated_at || "");
  const tb = Date.parse(b.updated_at || "");
  if (!Number.isNaN(ta) && !Number.isNaN(tb) && ta !== tb) return ta > tb;
  return false;
}

// A title one seller reuses for more than this many albums is a category
// label - a brand or garment type - not a product; live corpora show such
// groups holding 50-90 distinct products. Real duplicate postings appear
// once, at most twice, per seller.
const _TITLE_MAX_PER_SELLER = 2;

// Title keys used more than _TITLE_MAX_PER_SELLER times are category
// labels and never collapse. Call this over the FULL loaded corpus, not a
// filtered view: whether a title is a category label is a property of the
// seller's whole catalogue, and computing it on a filtered subset lets a
// banned group resurrect whenever filters hide enough of its members
// (for example, favourites view showing 2 of a seller's 60 same-title albums).
// Title keys already embed the seller, so one count per key is per-seller
// by construction; item-link identity is strong and never banned.
/**
 * @param {{dup_key?: string | null}[]} list
 * @returns {Set<string>}
 */
function bannedTitleKeys(list) {
  /** @type {Map<string, number>} */
  const counts = new Map();
  for (const album of list) {
    const key = album.dup_key;
    if (!key || !key.startsWith("t:")) continue;
    counts.set(key, (counts.get(key) || 0) + 1);
  }
  const banned = new Set();
  for (const [key, n] of counts) {
    if (n > _TITLE_MAX_PER_SELLER) banned.add(key);
  }
  return banned;
}

// Incremental form of bannedTitleKeys for streaming loads: fold one batch
// of freshly annotated albums into running per-key counts, promoting keys
// to the banned set the moment they cross the cap. Counts only ever grow,
// so a banned key never un-bans. Equivalent to recomputing
// bannedTitleKeys over everything seen so far, at O(batch) instead of
// O(corpus) per shard.
/**
 * @param {{dup_key?: string | null}[]} albums
 * @param {Map<string, number>} counts
 * @param {Set<string>} banned
 */
function registerTitleKeys(albums, counts, banned) {
  for (const album of albums) {
    const key = album.dup_key;
    if (!key || !key.startsWith("t:")) continue;
    const n = (counts.get(key) || 0) + 1;
    counts.set(key, n);
    if (n > _TITLE_MAX_PER_SELLER) banned.add(key);
  }
}

// Collapse a filtered album list to one representative per dup_key. Each
// group keeps its first occurrence's position; a better later member swaps
// in at that position. Albums with dup_key null or a banned title key pass
// through untouched. Pass the corpus-global banned set; the default only
// suits callers whose list IS the full corpus. Returns { out, counts }
// where counts maps dup_key -> group size.
/**
 * @template {DuplicateAlbum} T
 * @param {T[]} list
 * @param {Set<string>} [banned]
 * @returns {{out: T[], counts: Map<string, number>}}
 */
function collapseDups(list, banned = bannedTitleKeys(list)) {
  /** @type {T[]} */
  const out = [];
  /** @type {Map<string, number>} */
  const firstIdx = new Map();
  /** @type {Map<string, number>} */
  const counts = new Map();
  for (const album of list) {
    const key = album.dup_key;
    if (!key || banned.has(key)) { out.push(album); continue; }
    const idx = firstIdx.get(key);
    if (idx === undefined) {
      firstIdx.set(key, out.length);
      counts.set(key, 1);
      out.push(album);
    } else {
      counts.set(key, /** @type {number} */ (counts.get(key)) + 1);
      if (better(album, out[idx])) out[idx] = album;
    }
  }
  return { out, counts };
}

/**
 * Select over the complete loaded catalogue before applying display filters.
 * @template {DuplicateAlbum & {id: string, published_ms?: number, date_published?: string,
 * gone_since?: string}} T
 * @param {T[]} list
 */
function newestListings(list) {
  const banned = bannedTitleKeys(list);
  /** @type {Map<string, T>} */
  const winners = new Map();
  /** @param {T} album */
  const posted = album => {
    const known = typeof album.date_published === "string" ? album.date_published.slice(0, 10) : "";
    const ms = known && /^\d{4}-\d{2}-\d{2}$/.test(known) ? Date.parse(known + "T00:00:00Z") : NaN;
    if (Number.isFinite(ms) && new Date(ms).toISOString().slice(0, 10) === known && !known.startsWith("0000")) {
      return Math.floor(ms / 86400000);
    }
    const estimate = typeof module !== "undefined" ? require("./age.js").estimatePublishedMs : estimatePublishedMs;
    return Math.floor(Math.min(estimate(Number(album.id)), Date.UTC(9999, 11, 31)) / 86400000);
  };
  /** @param {T} album */
  const id = album => /^\d+$/.test(album.id) ? BigInt(album.id) : 0n;
  for (const album of list) {
    const key = album.dup_key;
    if (!key || banned.has(key)) continue;
    const current = winners.get(key);
    if (!current) {
      winners.set(key, album);
      continue;
    }
    const albumPosted = posted(album), currentPosted = posted(current);
    if (albumPosted !== currentPosted ? albumPosted > currentPosted : id(album) > id(current)) {
      winners.set(key, album);
    }
  }
  /** @type {Record<string, string>} */
  const aliases = {};
  const albums = list.filter(album => {
    const winner = album.dup_key && winners.get(album.dup_key);
    if (!winner || winner === album) return true;
    if (album.id !== winner.id) aliases[album.id] = winner.id;
    return false;
  });
  return {albums, aliases};
}

if (typeof module !== "undefined") {
  module.exports = { normTitle, titleWeight, SHORTLINK_HOSTS, marketplaceItem, marketplaceUrl,
    itemKeys, dupKey, better, bannedTitleKeys, registerTitleKeys, collapseDups, newestListings };
}
