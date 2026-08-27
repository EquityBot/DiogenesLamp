"use strict";

// Pure viewer transitions. DOM updates, scrolling, focus, storage and browser
// history remain effects in app.js; this module only decides the next state.

const _filterHelpers = typeof module !== "undefined" ? require("./filters.js") : null;

function filterDefaults() {
  return _filterHelpers ? _filterHelpers.defaultFilters() : defaultFilters();
}

function normalFilters(filters) {
  return _filterHelpers ? _filterHelpers.normalizeFilters(filters) : normalizeFilters(filters);
}

function createViewState(filters = filterDefaults()) {
  return {
    filters: normalFilters(filters),
    groupKey: null,
    preGroupScroll: 0,
    cursor: 0,
    shopsOpen: false,
  };
}

const FILTER_PARAMS = {
  search: "q",
  seller: "seller",
  category: "category",
  age: "age",
  sort: "sort",
  newOnly: "new",
  favsOnly: "favs",
  hideDups: "dups",
};
const GROUP_PARAM = "group";
const VIEW_PARAMS = new Set([...Object.values(FILTER_PARAMS), GROUP_PARAM]);

function asUrl(value) {
  if (value instanceof URL) return new URL(value.href);
  return new URL(String(value), "http://localhost/");
}

function hasViewStateParams(value) {
  let url;
  try {
    url = asUrl(value);
  } catch {
    return false;
  }
  return [...VIEW_PARAMS].some((name) => url.searchParams.has(name));
}

function stateFromUrl(value) {
  const url = asUrl(value);
  const raw = filterDefaults();
  for (const [field, param] of Object.entries(FILTER_PARAMS)) {
    if (!url.searchParams.has(param)) continue;
    if (typeof raw[field] === "boolean") {
      const value = url.searchParams.get(param);
      raw[field] = value === "1" || value === "true";
    } else {
      raw[field] = url.searchParams.get(param) || "";
    }
  }
  const state = createViewState(raw);
  const groupKey = url.searchParams.get(GROUP_PARAM);
  if (groupKey) state.groupKey = groupKey;
  return state;
}

function urlForViewState(value, state) {
  const url = asUrl(value);
  for (const param of VIEW_PARAMS) url.searchParams.delete(param);
  const filters = normalFilters(state && state.filters);
  const defaults = filterDefaults();
  for (const [field, param] of Object.entries(FILTER_PARAMS)) {
    const value = filters[field];
    if (value === defaults[field]) continue;
    url.searchParams.set(param, typeof value === "boolean" ? "1" : value);
  }
  if (state && typeof state.groupKey === "string" && state.groupKey) {
    url.searchParams.set(GROUP_PARAM, state.groupKey);
  }
  return url;
}

function nextViewState(state, event) {
  if (!event || typeof event.type !== "string") return state;
  if (event.type === "filter") {
    return {
      ...state,
      filters: normalFilters(event.filters),
      groupKey: null,
      cursor: 0,
    };
  }
  if (event.type === "search") {
    return {
      ...state,
      filters: normalFilters({
        ...state.filters,
        search: typeof event.value === "string" ? event.value : "",
      }),
      groupKey: null,
      cursor: 0,
    };
  }
  if (event.type === "clear-filters") {
    return {
      ...state,
      filters: filterDefaults(),
      groupKey: null,
      preGroupScroll: 0,
      cursor: 0,
    };
  }
  if (event.type === "open-group") {
    if (typeof event.groupKey !== "string" || !event.groupKey) return state;
    const scrollY = Number(event.scrollY);
    return {
      ...state,
      groupKey: event.groupKey,
      preGroupScroll: Number.isFinite(scrollY) ? Math.max(0, scrollY) : 0,
    };
  }
  if (event.type === "close-group") {
    return { ...state, groupKey: null, cursor: 0 };
  }
  if (event.type === "toggle-shops") {
    return { ...state, shopsOpen: !state.shopsOpen };
  }
  if (event.type === "set-shops") {
    return { ...state, shopsOpen: Boolean(event.open) };
  }
  if (event.type === "cursor") {
    const cursor = Number(event.index);
    if (!Number.isInteger(cursor) || cursor < 0) return state;
    return { ...state, cursor };
  }
  if (event.type === "escape") {
    if (state.shopsOpen) return { ...state, shopsOpen: false };
    if (state.groupKey) return { ...state, groupKey: null, cursor: 0 };
    return state;
  }
  if (event.type === "url") {
    try {
      return stateFromUrl(event.url);
    } catch {
      return state;
    }
  }
  return state;
}

if (typeof module !== "undefined") {
  module.exports = {
    createViewState,
    hasViewStateParams,
    nextViewState,
    urlForViewState,
  };
}
