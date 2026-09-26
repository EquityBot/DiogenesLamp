"use strict";

/** @typedef {{id: string, title?: string, title_lc?: string, seller?: string,
 * links?: import("./dedup.js").PurchaseLink[], search_ids?: string[],
 * search_keys?: string[]}} SearchAlbum */
const SEARCH_DEDUP = typeof module !== "undefined" ? require("./dedup.js") : { itemKeys };

/** @param {SearchAlbum} album */
function prepareAlbumSearch(album) {
  const keys = [...SEARCH_DEDUP.itemKeys(album.links || [])];
  album.title_lc = (album.title || "").normalize("NFKC").toLowerCase();
  album.search_keys = keys;
  album.search_ids = [album.id, ...keys.filter(key => !key.startsWith("url:"))
    .map(key => key.slice(key.indexOf(":") + 1))];
}

/** @param {string} query */
function compileCatalogueSearch(query) {
  const text = query.trim().normalize("NFKC");
  let exactId = /^\d+$/.test(text) ? text : "";
  let exactKey = "";
  let isUrl = false;
  try {
    const url = new URL(text);
    isUrl = true;
    const host = url.hostname.toLowerCase();
    if (host.endsWith(".yupoo.com") || host === "yupoo.com") {
      exactId = /^\/albums\/(\d+)\/?$/.exec(url.pathname)?.[1] || "";
    } else {
      const platform = host === "weidian.com" || host.endsWith(".weidian.com") ? "weidian"
        : host === "tmall.com" || host.endsWith(".tmall.com") ? "tmall"
        : host === "taobao.com" || host.endsWith(".taobao.com") ? "taobao"
        : ["m.tb.cn", "k.youshop10.com"].includes(host) ? "url" : "";
      if (platform) exactKey = [...SEARCH_DEDUP.itemKeys([{platform, url: url.href}])][0] || "";
    }
  } catch { /* Ordinary text search. */ }
  const terms = text.toLowerCase().split(/\s+/).filter(Boolean);
  /** @param {SearchAlbum} album */
  function reason(album) {
    if (!text) return "";
    if (exactKey) return album.search_keys?.includes(exactKey) ? "Purchase link" : null;
    if (exactId) return (album.search_ids || [album.id]).includes(exactId) ? "Exact item ID" : null;
    if (isUrl) return null;
    const title = album.title_lc || (album.title || "").normalize("NFKC").toLowerCase();
    const seller = (album.seller || "").normalize("NFKC").toLowerCase();
    if (terms.every(term => title.includes(term))) return "Title";
    if (terms.every(term => (title + " " + seller).includes(term))) return "Title or seller";
    return null;
  }
  /** @param {SearchAlbum} album */
  const matches = album => reason(album) !== null;
  return { reason, matches };
}

/** @param {unknown} value */
function csvCell(value) {
  let text = String(value ?? "");
  if (/^[\s]*[=+@-]/.test(text) || /^[\t\r\n]/.test(text)) text = "'" + text;
  return '"' + text.replace(/"/g, '""') + '"';
}

/** @param {Record<string, any>[]} albums @param {string} status */
function catalogueCsv(albums, status) {
  const rows = [["seller","album_id","title","currency","price","details_checked","album_url",
    "purchase_links","export_status"]];
  for (const a of albums) rows.push([a.seller,a.id,a.title,"CNY",a.price || "",a.updated_at || "",
    a.album_url,(a.links || []).map(/** @param {{url: string}} link */ link => link.url).join(" "),status]);
  return "\uFEFF" + rows.map(row => row.map(csvCell).join(",")).join("\r\n") + "\r\n";
}

/** @param {number} value */
function bitCount(value) {
  value -= (value >>> 1) & 0x55555555;
  value = (value & 0x33333333) + ((value >>> 2) & 0x33333333);
  return (((value + (value >>> 4)) & 0x0f0f0f0f) * 0x01010101) >>> 24;
}

/**
 * Opt-in visual suggestions only. A dHash measures layout, not product identity.
 * ponytail: a bounded linear scan is sufficient for one preview at a time;
 * use an indexed search only if measured catalogue sizes exceed this ceiling.
 * @template {{id: string, cover_hash?: string}} T
 * @param {T[]} albums @param {T} source
 * @param {Set<string>} [dismissed] @param {number} [limit]
 */
function similarCovers(albums, source, dismissed = new Set(), limit = 8) {
  const valid = /** @param {string | undefined} h */ h =>
    typeof h === "string" && /^[0-9a-f]{16}$/.test(h) && !/^(0{16}|f{16})$/.test(h);
  if (!valid(source.cover_hash)) return [];
  const hash = source.cover_hash || "";
  const hi = parseInt(hash.slice(0, 8), 16), lo = parseInt(hash.slice(8), 16);
  /** @type {{album: T, distance: number}[]} */
  const matches = [];
  const cap = Math.min(20, Math.max(1, limit));
  for (const album of albums.slice(0, 100000)) {
    if (album.id === source.id || dismissed.has(album.id) || !valid(album.cover_hash)) continue;
    const other = album.cover_hash || "";
    const distance = bitCount(hi ^ parseInt(other.slice(0, 8), 16)) + bitCount(lo ^ parseInt(other.slice(8), 16));
    if (distance > 8) continue;
    matches.push({album, distance});
    matches.sort((a, b) => a.distance - b.distance || a.album.id.localeCompare(b.album.id));
    if (matches.length > cap) matches.pop();
  }
  return matches;
}

/** @param {unknown} raw @param {string} seller */
function validCatalogueShard(raw, seller) {
  if (!raw || typeof raw !== "object") return false;
  const shard = /** @type {Record<string, any>} */ (raw);
  if (![undefined, 1].includes(shard.schema_version) || !Array.isArray(shard.albums)) return false;
  const ids = new Set();
  const safeURL = /** @param {unknown} value */ value => {
    try {
      if (typeof value !== "string") return false;
      const parsed = new URL(value);
      return parsed.protocol === "https:" && !parsed.username && !parsed.password;
    }
    catch { return false; }
  };
  for (const album of shard.albums) {
    if (!album || typeof album.id !== "string" || !/^\d{1,24}$/.test(album.id) || ids.has(album.id) ||
        album.seller !== seller || typeof album.title !== "string" || !Array.isArray(album.links) ||
        !safeURL(album.album_url) || typeof album.thumb !== "string" ||
        (album.thumb && !/^thumbs\/[a-z0-9_-]+\/[0-9]+\.(jpg|png|webp)$/.test(album.thumb)) ||
        album.links.some(/** @param {any} link */ link => !link || typeof link.platform !== "string" || !safeURL(link.url))) return false;
    if (album.price !== undefined && (typeof album.price !== "number" || !Number.isFinite(album.price) || album.price < 0)) return false;
    if (album.history !== undefined && (!Array.isArray(album.history) || album.history.some(
      /** @param {any} event */ event => !event || typeof event.at !== "string"))) return false;
    if (album.price_evidence?.candidates !== undefined && !Array.isArray(album.price_evidence.candidates)) return false;
    if (album.cover_hash !== undefined && typeof album.cover_hash !== "string") return false;
    ids.add(album.id);
  }
  return true;
}

if (typeof module !== "undefined") {
  module.exports = { prepareAlbumSearch, compileCatalogueSearch, csvCell, catalogueCsv,
    similarCovers, validCatalogueShard };
}
