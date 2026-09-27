"use strict";

/**
 * @typedef {import("./filters.js").Filters} Filters
 * @typedef {{filters: Filters, storedFilters: Filters, userTouchedFilters: boolean,
 *   groupKey: string | null, preGroupScroll: number, cursor: number}} ViewState
 * @typedef {"seller" | "category" | "sort"} RestoredOption
 * @typedef {{type: "filter", filters: Filters} | {type: "clear-filters"}
 *   | {type: "restore-options", options: Partial<Record<RestoredOption, string[]>>}
 *   | {type: "open-group", groupKey: string, scrollY: number} | {type: "close-group"}
 *   | {type: "cursor", index: number, count: number}} ViewEvent
 * @typedef {import("./filters.js").SortableAlbum & import("./dedup.js").DuplicateAlbum
 *   & {seller: string, category: string, first_seen: string, published_ms: number,
 *      title_lc: string, is_new?: boolean, gone_since?: string, updated_at?: string,
 *      history?: {at: string}[], search_ids?: string[], search_keys?: string[],
 *      kind?: string}} CatalogueAlbum
 */

// Catalogue choices and transitions. app.js owns DOM, storage, scroll and focus.
const VIEW_HELPERS = typeof module !== "undefined" ? {
  ...require("./filters.js"),
  ...require("./catalogue.js"),
  ...require("./dedup.js"),
  ...require("./age.js"),
  ...require("./discovery.js"),
} : { normalizeFilters, defaultFilters, matchSearch, sortAlbums, clampCursor,
  collapseDups, ageInDays, compileCatalogueSearch };

/**
 * @param {unknown} [storedFilters]
 * @param {unknown} [filters]
 * @returns {ViewState}
 */
function createViewState(storedFilters = {}, filters = storedFilters) {
  return {
    filters: VIEW_HELPERS.normalizeFilters(filters),
    storedFilters: VIEW_HELPERS.normalizeFilters(storedFilters),
    userTouchedFilters: false,
    groupKey: null,
    preGroupScroll: 0,
    cursor: 0,
  };
}

/**
 * @param {ViewState} view
 * @param {ViewEvent} event
 * @returns {ViewState}
 */
function nextViewState(view, event) {
  switch (event.type) {
    case "filter":
    case "clear-filters":
      return { ...view, userTouchedFilters: true, groupKey: null, cursor: 0,
        filters: event.type === "clear-filters" ? VIEW_HELPERS.defaultFilters()
          : VIEW_HELPERS.normalizeFilters(event.filters) };
    case "restore-options": {
      if (view.userTouchedFilters) return view;
      const filters = { ...view.filters };
      for (const key of /** @type {RestoredOption[]} */ (["seller", "category", "sort"])) {
        const saved = view.storedFilters[key];
        filters[key] = saved && (event.options[key] || []).includes(saved)
          ? saved : VIEW_HELPERS.defaultFilters()[key];
      }
      return { ...view, filters };
    }
    case "open-group":
      return { ...view, groupKey: event.groupKey,
        preGroupScroll: Number.isFinite(event.scrollY) ? Math.max(0, event.scrollY) : 0 };
    case "close-group":
      return { ...view, groupKey: null, cursor: 0 };
    case "cursor":
      return { ...view, cursor: VIEW_HELPERS.clampCursor(event.index, event.count) };
    default:
      return view;
  }
}

/**
 * @template {CatalogueAlbum} T
 * @param {ViewState} view
 * @param {{albums: T[], favorites: Set<string>, bannedKeys: Set<string>,
 * notes?: Map<string, unknown>, collections?: Map<string, unknown>, tags?: Map<string, unknown>,
 * reviews?: Map<string, unknown>}} catalogue
 * @param {number} [now]
 */
function selectCatalogue(view, catalogue, now = Date.now()) {
  const { albums, favorites, bannedKeys } = catalogue;
  const filters = view.filters;
  let filtered;
  /** @type {Map<string, number>} */
  let dupCounts = new Map();
  let dupHidden = 0;
  if (view.groupKey) {
    filtered = albums.filter(album => album.dup_key === view.groupKey);
  } else {
    const matchesSearch = VIEW_HELPERS.compileCatalogueSearch(filters.search).matches;
    const min = filters.priceMin ? Number(filters.priceMin) : 0;
    const max = filters.priceMax ? Number(filters.priceMax) : Infinity;
    const priceMatches = /** @param {number | null | undefined} price */ price =>
      min <= max && (price ? price >= min && price <= max : filters.includeUnknown);
    const reviewed = /** @param {T} album */ album => Number(catalogue.reviews?.get(album.seller) || 0);
    const maxDays = parseInt(filters.age, 10) || 0;
    // An album deleted on Yupoo leads to a dead page, so it drops out of
    // browsing. Saved filters and change history keep it available for reference.
    filtered = albums.filter(album =>
      (!album.gone_since || filters.favsOnly || !!filters.collection || !!filters.tag || filters.changed) &&
      (!filters.seller || album.seller === filters.seller) &&
      (!filters.category || album.category === filters.category) &&
      (!filters.kind || album.kind === filters.kind) &&
      (!filters.newOnly || album.is_new) &&
      (!filters.favsOnly || favorites.has(album.id)) &&
      (!maxDays || VIEW_HELPERS.ageInDays(album.published_ms, now) <= maxDays) &&
      priceMatches(album.price) &&
      (!filters.collection || catalogue.collections?.get(album.id) === filters.collection) &&
      (!filters.tag || /** @type {string[]} */ (catalogue.tags?.get(album.id) || []).includes(filters.tag)) &&
      (!filters.unreviewed || Date.parse(album.first_seen) > reviewed(album)) &&
      (!filters.changed || (album.history || []).some(event => Date.parse(event.at) > reviewed(album))) &&
      matchesSearch(album)
    );
    if (filters.hideDups) {
      const { out, counts } = VIEW_HELPERS.collapseDups(filtered, bannedKeys);
      dupHidden = filtered.length - out.length;
      filtered = out;
      dupCounts = counts;
    }
  }
  filtered = VIEW_HELPERS.sortAlbums(filtered, filters.sort);
  return { filtered, dupCounts, dupHidden,
    cursor: VIEW_HELPERS.clampCursor(view.cursor, filtered.length) };
}

if (typeof module !== "undefined") {
  module.exports = { createViewState, nextViewState, selectCatalogue };
}
