"use strict";

// Filter state is saved so a reload puts the user back where they were,
// and every localStorage access the viewer makes goes through this file.
// Nothing here touches the DOM: the browser passes in a plain object and
// gets one back, which is what makes it testable under node.

const FILTER_STORAGE_KEY = "lamp.filters";

// Every persisted field, with the value that means "not filtering". A field
// at its default is dropped on save, so a stored blob stays small and an
// added filter defaults sensibly for someone with an old blob saved.
const FILTER_DEFAULTS = {
  search: "",
  seller: "",
  category: "",
  age: "",
  sort: "default",
  newOnly: false,
  favsOnly: false,
  hideDups: false,
};

function defaultFilters() {
  return { ...FILTER_DEFAULTS };
}

// Coerce whatever was stored into the shape the viewer expects. Anything
// unrecognised falls back to the default rather than being trusted: the blob
// is editable by hand, and a string where a boolean belongs would otherwise
// make `checked` truthy for any non-empty value.
function normalizeFilters(raw) {
  const out = defaultFilters();
  if (!raw || typeof raw !== "object") return out;
  for (const [key, fallback] of Object.entries(FILTER_DEFAULTS)) {
    const value = raw[key];
    if (typeof fallback === "boolean") {
      if (typeof value === "boolean") out[key] = value;
    } else if (typeof value === "string") {
      out[key] = value;
    }
  }
  return out;
}

// True when at least one filter is doing something, which is what decides
// whether the "Clear filters" control is offered.
function anyFilterActive(filters) {
  const state = normalizeFilters(filters);
  return Object.entries(FILTER_DEFAULTS)
    .some(([key, fallback]) => state[key] !== fallback);
}

// Drop defaults before storing. Restoring a blob written by an older build
// then leaves new fields at their default instead of undefined.
function serializeFilters(filters) {
  const state = normalizeFilters(filters);
  const out = {};
  for (const [key, fallback] of Object.entries(FILTER_DEFAULTS)) {
    if (state[key] !== fallback) out[key] = state[key];
  }
  return out;
}

// These take the host - the window, or any object carrying a localStorage -
// rather than a Storage, and reach the accessor themselves. The reason is
// that `window.localStorage` is a getter, and a browser with site data
// blocked for the origin raises SecurityError from the getter, before
// getItem is ever reached. An argument is evaluated at the call site, so
// `readStored(localStorage, key)` throws there, outside this function's try,
// and guards nothing; only the host survives the trip. A host carrying no
// storage at all reads as "no value" instead of throwing, the same as a
// blocked one.
//
// app.js persists favourites, the filter blob and the colour theme, and
// reads all three while its module body is still executing - an exception
// there binds no listeners, loads no catalog and leaves a blank page
// recoverable only by clearing site data.
function readStored(host, key) {
  try {
    return host.localStorage.getItem(key);
  } catch {
    return null;
  }
}

function writeStored(host, key, value) {
  try {
    host.localStorage.setItem(key, value);
  } catch {
    // Persistence is a convenience; the session still honours the choice.
  }
}

// A corrupt or unparseable blob must not throw: this runs during start-up,
// and an exception here would leave the viewer with no listeners bound. The
// host is passed rather than the Storage for the reason given above.
function readFilters(host) {
  try {
    const raw = host.localStorage.getItem(FILTER_STORAGE_KEY);
    if (!raw) return defaultFilters();
    return normalizeFilters(JSON.parse(raw));
  } catch {
    return defaultFilters();
  }
}

function writeFilters(host, filters) {
  try {
    host.localStorage.setItem(FILTER_STORAGE_KEY, JSON.stringify(serializeFilters(filters)));
  } catch {
    // A full or blocked quota costs the user persistence, not the session.
  }
}

// Multi-term title matching: all whitespace-separated terms in query must
// appear in the title (case-insensitive, in any order).
function matchSearch(titleLc, query) {
  if (!query) return true;
  const terms = query.trim().toLowerCase().split(/\s+/).filter(Boolean);
  if (!terms.length) return true;
  const target = (titleLc || "").toLowerCase();
  return terms.every((term) => target.includes(term));
}

// Global sort. "Newest first" must order the whole catalog, not leave albums
// grouped in seller-shard order. Missing prices sort last on low-to-high and
// first-to-last on high-to-low (treated as 0).
function sortAlbums(albums, sortOrder) {
  const out = albums.slice();
  if (sortOrder === "price-asc") {
    out.sort((a, b) => (a.price || Infinity) - (b.price || Infinity));
  } else if (sortOrder === "price-desc") {
    out.sort((a, b) => (b.price || 0) - (a.price || 0));
  } else if (sortOrder === "photos-desc") {
    out.sort((a, b) => (b.photo_count || 0) - (a.photo_count || 0));
  } else {
    out.sort((a, b) => {
      const byDate = (b.published_ms || 0) - (a.published_ms || 0);
      if (byDate) return byDate;
      return (Number(b.id) || 0) - (Number(a.id) || 0);
    });
  }
  return out;
}

function formatPrice(price) {
  if (!price) return "";
  return `¥${price}`;
}

// "/" focuses search unless the user is already typing in a field.
function isSearchShortcut(key, activeTag) {
  if (key !== "/") return false;
  const tag = (activeTag || "").toLowerCase();
  return tag !== "input" && tag !== "select" && tag !== "textarea";
}

// Favourites export must not silently drop stars whose shards are still
// loading or failed. Callers show `reason` and `omittedIds` to the user.
function favouritesExportPlan({ favoriteIds, albums, loading }) {
  const favSet = favoriteIds instanceof Set ? favoriteIds : new Set(favoriteIds || []);
  if (!favSet.size) {
    return { ready: false, reason: "empty", albums: [], omittedIds: [] };
  }
  if (loading) {
    return { ready: false, reason: "loading", albums: [], omittedIds: [...favSet] };
  }
  const byId = new Map((albums || []).map((album) => [album.id, album]));
  const exported = [];
  const omittedIds = [];
  for (const id of favSet) {
    const album = byId.get(id);
    if (album) exported.push(album);
    else omittedIds.push(id);
  }
  return {
    ready: exported.length > 0,
    reason: omittedIds.length ? (exported.length ? "partial" : "missing") : "ok",
    albums: exported,
    omittedIds,
  };
}

if (typeof module !== "undefined") {
  module.exports = { FILTER_STORAGE_KEY, FILTER_DEFAULTS, defaultFilters,
    normalizeFilters, anyFilterActive, serializeFilters, readStored,
    writeStored, readFilters, writeFilters, matchSearch, sortAlbums,
    formatPrice, isSearchShortcut, favouritesExportPlan };
}
