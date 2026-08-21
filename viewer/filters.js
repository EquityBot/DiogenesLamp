"use strict";

// Filter state is saved so a reload puts the user back where they were.
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

// A corrupt or unparseable blob must not throw: this runs during start-up,
// and an exception here would leave the viewer with no listeners bound.
function readFilters(storage) {
  try {
    const raw = storage.getItem(FILTER_STORAGE_KEY);
    if (!raw) return defaultFilters();
    return normalizeFilters(JSON.parse(raw));
  } catch {
    return defaultFilters();
  }
}

function writeFilters(storage, filters) {
  try {
    storage.setItem(FILTER_STORAGE_KEY, JSON.stringify(serializeFilters(filters)));
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

if (typeof module !== "undefined") {
  module.exports = { FILTER_STORAGE_KEY, FILTER_DEFAULTS, defaultFilters,
    normalizeFilters, anyFilterActive, serializeFilters, readFilters,
    writeFilters, matchSearch };
}
