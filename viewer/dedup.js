"use strict";

// Duplicate-posting detection. Pure functions only - no DOM access - so the
// same file runs in the browser (script tag before app.js) and under node
// for tests.

// Titles compare after NFKC folding (fullwidth yen/digits -> ASCII), case
// folding, and stripping everything that is not a letter or digit. CJK
// characters are letters and survive.
function normTitle(title) {
  return (title || "")
    .normalize("NFKC")
    .toLowerCase()
    .replace(/[^\p{L}\p{N}]+/gu, "");
}

// One identity key per link that points at a single purchasable item:
//   - taobao/tmall/weidian item pages carry ?id= / ?itemID=
//   - m.tb.cn and k.youshop10.com shortlinks identify the item by path token
// Shop/homepage links (weidian ?userid=, bare domains) identify the seller,
// not an item, and contribute nothing - otherwise every album that repeats
// the seller's shop link would collapse into one group.
// URL() does the parsing so stray fragments ("?id=695#detail" pasted from
// album text) and encoding never leak into the key, and a link entry whose
// url is missing or malformed is skipped instead of throwing - one bad
// link must not cost the whole shard.
function itemKeys(links) {
  const keys = new Set();
  for (const link of links) {
    if (!link || typeof link.url !== "string") continue;
    let url;
    try { url = new URL(link.url); } catch { continue; }
    let id = "";
    for (const [k, v] of url.searchParams) {
      const kl = k.toLowerCase();
      if (v && (kl === "id" || kl === "itemid")) { id = v; break; }
    }
    if (id) { keys.add(`${link.platform}:${id}`); continue; }
    if ((url.hostname === "m.tb.cn" || url.hostname === "k.youshop10.com") &&
        url.pathname.length > 1) {
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

function titleWeight(normalized) {
  let weight = 0;
  for (const ch of normalized) {
    weight += ch.codePointAt(0) > 0x2e7f ? 2 : 1;
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
function dupKey(album) {
  const keys = itemKeys(album.links || []);
  if (keys.size === 1) return `i:${keys.values().next().value}`;
  const title = normTitle(album.title);
  if (titleWeight(title) < _TITLE_MIN_WEIGHT) return null;
  return `t:${album.seller}:${title}`;
}

// True when a should represent its group instead of b: more purchase links
// wins, then newer updated_at; otherwise keep the incumbent (stable order).
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
function bannedTitleKeys(list) {
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
function collapseDups(list, banned = bannedTitleKeys(list)) {
  const out = [];
  const firstIdx = new Map();
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
      counts.set(key, counts.get(key) + 1);
      if (better(album, out[idx])) out[idx] = album;
    }
  }
  return { out, counts };
}

if (typeof module !== "undefined") {
  module.exports = { normTitle, titleWeight, itemKeys, dupKey, better,
    bannedTitleKeys, registerTitleKeys, collapseDups };
}
