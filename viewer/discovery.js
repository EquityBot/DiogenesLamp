"use strict";

/** @typedef {{id: string, title?: string, title_lc?: string, seller?: string,
 * description?: string, description_lc?: string,
 * links?: import("./dedup.js").PurchaseLink[], search_ids?: string[],
 * search_keys?: string[]}} SearchAlbum */
const SEARCH_DEDUP = typeof module !== "undefined" ? require("./dedup.js")
  : { itemKeys, marketplaceItem, SHORTLINK_HOSTS };
const SEARCH_AGENTS = typeof module !== "undefined" ? require("./agents.js")
  : { unwrapAgentLink, agentLink, agentFor };

/** @param {SearchAlbum} album */
function prepareAlbumSearch(album) {
  const keys = [...SEARCH_DEDUP.itemKeys(album.links || [])];
  album.title_lc = (album.title || "").normalize("NFKC").toLowerCase();
  album.description_lc = (album.description || "").normalize("NFKC").toLowerCase();
  album.search_keys = keys;
  album.search_ids = [album.id, ...keys.filter(key => !key.startsWith("url:"))
    .map(key => key.slice(key.indexOf(":") + 1))];
}

// The dedup key a pasted or spreadsheet URL resolves to: an agent link is
// unwrapped first, a marketplace item keys on its platform and id, a
// shortlink on its path. Anything else, including a shop link, is "".
/** @param {string} raw */
function keyForUrl(raw) {
  const unwrapped = SEARCH_AGENTS.unwrapAgentLink(raw);
  const item = SEARCH_DEDUP.marketplaceItem(unwrapped || raw);
  if (item) return `${item.platform}:${item.id}`;
  try {
    const url = new URL(raw);
    if (SEARCH_DEDUP.SHORTLINK_HOSTS.includes(url.hostname) && url.pathname.length > 1) {
      return `url:${url.hostname}${url.pathname}`;
    }
  } catch { /* not a URL */ }
  return "";
}

// Album text carries purchase links without a scheme and the scraper reads
// them; a pasted copy of one gets its scheme back here, but only when the
// host is one the catalogue keys on, so ordinary words with a dot stay a
// text search. Throws like new URL() for anything else.
/** @param {string} text */
function pastedUrl(text) {
  try { return new URL(text); } catch { /* try a scheme-less known host */ }
  if (!/^[a-z0-9.-]+\.[a-z]{2,}(?:[/?]|$)/i.test(text)) throw new TypeError("not a URL");
  const url = new URL("https://" + text);
  const host = url.hostname.toLowerCase();
  const known = host === "yupoo.com" || host.endsWith(".yupoo.com") ||
    SEARCH_DEDUP.SHORTLINK_HOSTS.includes(host) || SEARCH_AGENTS.agentFor(host) !== null ||
    SEARCH_DEDUP.marketplaceItem(url.href) !== null;
  if (!known) throw new TypeError("not a known host");
  return url;
}

/** @param {string} query */
function compileCatalogueSearch(query) {
  const text = query.trim().normalize("NFKC");
  let exactId = /^\d+$/.test(text) ? text : "";
  let exactKey = "";
  let isUrl = false;
  try {
    const url = pastedUrl(text);
    isUrl = true;
    const host = url.hostname.toLowerCase();
    if (host.endsWith(".yupoo.com") || host === "yupoo.com") {
      exactId = /^\/albums\/(\d+)\/?$/.exec(url.pathname)?.[1] || "";
    } else {
      exactKey = keyForUrl(url.href);
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
    const description = album.description_lc ?? (album.description || "").normalize("NFKC").toLowerCase();
    if (description && terms.every(term => (title + "\n" + description).includes(term))) return "Description";
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

// The chosen agent's link for each of an album's marketplace items, in
// link order; empty when no agent is chosen or no item has a template.
/** @param {Record<string, any>} album @param {string} agentId @returns {string[]} */
function albumAgentLinks(album, agentId) {
  if (!agentId) return [];
  return (album.links || [])
    .map(/** @param {{url?: unknown}} link */ link =>
      SEARCH_AGENTS.agentLink(agentId, SEARCH_DEDUP.marketplaceItem(link?.url)))
    .filter(Boolean);
}

/** @param {Record<string, any>[]} albums @param {string} status @param {string} [agentId] */
function catalogueCsv(albums, status, agentId = "") {
  const rows = [["seller","album_id","title","currency","price","details_checked","album_url",
    "purchase_links","export_status","description","kind","agent_links"]];
  for (const a of albums) rows.push([a.seller,a.id,a.title,"CNY",a.price || "",a.updated_at || "",
    a.album_url,(a.links || []).map(/** @param {{url: string}} link */ link => link.url).join(" "),status,
    a.description || "", a.kind || "", albumAgentLinks(a, agentId).join(" ")]);
  return "\uFEFF" + rows.map(row => row.map(csvCell).join(",")).join("\r\n") + "\r\n";
}

// The delimiter a sheet uses: comma, semicolon or tab, whichever occurs
// most in the first record, counted outside quoted cells so a quoted
// "a,b,c" cannot pass a semicolon sheet off as a comma one.
/** @param {string} body */
function detectDelimiter(body) {
  const counts = { ",": 0, ";": 0, "\t": 0 };
  let quoted = false;
  for (let i = 0; i < body.length; i++) {
    const ch = body[i];
    if (quoted) { if (ch === '"') quoted = body[i + 1] === '"' ? (i++, true) : false; }
    else if (ch === '"') quoted = true;
    else if (ch === "\n" || ch === "\r") break;
    else if (ch in counts) counts[/** @type {keyof typeof counts} */ (ch)] += 1;
  }
  return /** @type {(keyof typeof counts)[]} */ ([",", ";", "\t"])
    .reduce((best, candidate) => counts[candidate] > counts[best] ? candidate : best, ",");
}

// Split delimited text into rows of cells: RFC 4180 quoting, CRLF or LF,
// an optional BOM, and the delimiter read from the first record. A
// community spreadsheet export is the input, so nothing about its columns
// is assumed.
/** @param {string} text @returns {string[][]} */
function parseDelimited(text) {
  const body = text.replace(/^\uFEFF/, "");
  const delimiter = detectDelimiter(body);
  /** @type {string[][]} */
  const rows = [];
  /** @type {string[]} */
  let row = [];
  let cell = "";
  let quoted = false;
  for (let i = 0; i < body.length; i++) {
    const ch = body[i];
    if (quoted) {
      if (ch === '"') {
        if (body[i + 1] === '"') { cell += '"'; i++; } else quoted = false;
      } else cell += ch;
    } else if (ch === '"' && cell === "") quoted = true;
    else if (ch === delimiter) { row.push(cell); cell = ""; }
    else if (ch === "\n" || ch === "\r") {
      if (ch === "\r" && body[i + 1] === "\n") i++;
      row.push(cell); rows.push(row); row = []; cell = "";
    } else cell += ch;
  }
  if (cell !== "" || row.length) { row.push(cell); rows.push(row); }
  return rows;
}

// A candidate starts at a URL boundary and its host ends at one, so a
// lookalike such as evil-taobao.com never yields its suffix as a match.
const SHEET_URL = /(?<![a-z0-9.@-])(?:https?:\/\/)?(?:[a-z0-9-]+\.)*(?:taobao\.com|tmall\.com|weidian\.com|1688\.com|m\.tb\.cn|k\.youshop10\.com|[a-z0-9-]+buy\.com|cnfans\.com|joyagoo\.com|sugargoo\.com)(?!\.?[a-z0-9-]|@)(?:\/[^\s"'<>]*|\?[^\s"'<>]*)?/gi;

/**
 * Every purchasable item a spreadsheet names, keyed the way albums are:
 * each cell is scanned for marketplace, shortlink and agent URLs, agent
 * links are unwrapped, and the keys come back with the row and URL
 * counts so the operator can see how much of the sheet was usable.
 * @param {string} text
 * @param {number} [maxRows]
 */
function sheetItemKeys(text, maxRows = 100000) {
  const rows = parseDelimited(text);
  if (rows.length > maxRows) throw new Error(`The spreadsheet has more than ${maxRows} rows.`);
  const keys = new Set();
  let urls = 0;
  for (const row of rows) {
    for (const cell of row) {
      for (const match of cell.matchAll(SHEET_URL)) {
        urls++;
        const raw = /^https?:\/\//i.test(match[0]) ? match[0] : "https://" + match[0];
        const key = keyForUrl(raw.replace(/[.,;:!?)\]}>]+$/, ""));
        if (key) keys.add(key);
      }
    }
  }
  return { keys, rows: rows.length, urls };
}

/**
 * The albums whose item keys a sheet names, and every named key that was
 * found: an album with two links in the sheet counts once, both keys count.
 * @template {{id: string, search_keys?: string[]}} T
 * @param {T[]} albums @param {Set<string>} keys
 */
function matchSheet(albums, keys) {
  const found = new Set();
  /** @type {T[]} */
  const matched = [];
  for (const album of albums) {
    const hits = (album.search_keys || []).filter(key => keys.has(key));
    if (!hits.length) continue;
    matched.push(album);
    for (const key of hits) found.add(key);
  }
  return { matched, found };
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
  const hi = Number.parseInt(hash.slice(0, 8), 16), lo = Number.parseInt(hash.slice(8), 16);
  /** @type {{album: T, distance: number}[]} */
  const matches = [];
  const cap = Math.min(20, Math.max(1, limit));
  for (const album of albums.slice(0, 100000)) {
    if (album.id === source.id || dismissed.has(album.id) || !valid(album.cover_hash)) continue;
    const other = album.cover_hash || "";
    const distance = bitCount(hi ^ Number.parseInt(other.slice(0, 8), 16)) + bitCount(lo ^ Number.parseInt(other.slice(8), 16));
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
    if (album.description !== undefined && typeof album.description !== "string") return false;
    if (album.description_truncated !== undefined && typeof album.description_truncated !== "boolean") return false;
    ids.add(album.id);
  }
  return true;
}

if (typeof module !== "undefined") {
  module.exports = { prepareAlbumSearch, keyForUrl, compileCatalogueSearch, csvCell, albumAgentLinks,
    catalogueCsv, parseDelimited, sheetItemKeys, matchSheet,
    similarCovers, validCatalogueShard };
}
