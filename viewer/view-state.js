"use strict";

// Catalogue choices and transitions. app.js owns DOM, storage, scroll and focus.
const VIEW_HELPERS = typeof module !== "undefined" ? {
  ...require("./filters.js"),
  ...require("./catalogue.js"),
  ...require("./dedup.js"),
  ...require("./age.js"),
} : { normalizeFilters, defaultFilters, matchSearch, sortAlbums, clampCursor,
  collapseDups, ageInDays };

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
      for (const key of ["seller", "category", "sort"]) {
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

function selectCatalogue(view, catalogue, now = Date.now()) {
  const { albums, favorites, generatedAt, bannedKeys } = catalogue;
  const filters = view.filters;
  let filtered;
  let dupCounts = new Map();
  let dupHidden = 0;
  if (view.groupKey) {
    filtered = albums.filter(album => album.dup_key === view.groupKey);
  } else {
    const matchesSearch = VIEW_HELPERS.matchSearch(filters.search);
    const maxDays = parseInt(filters.age, 10) || 0;
    filtered = albums.filter(album =>
      (!filters.seller || album.seller === filters.seller) &&
      (!filters.category || album.category === filters.category) &&
      (!filters.newOnly || album.first_seen === generatedAt) &&
      (!filters.favsOnly || favorites.has(album.id)) &&
      (!maxDays || VIEW_HELPERS.ageInDays(album.published_ms, now) <= maxDays) &&
      matchesSearch(album.title_lc)
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
