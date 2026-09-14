// The script tags in index.html expose these helpers to view-state.js.
// Reuse the CommonJS export types so both loading paths share one contract.
declare const normalizeFilters: typeof import("./filters.js").normalizeFilters;
declare const defaultFilters: typeof import("./filters.js").defaultFilters;
declare const matchSearch: typeof import("./filters.js").matchSearch;
declare const sortAlbums: typeof import("./filters.js").sortAlbums;
declare const clampCursor: typeof import("./catalogue.js").clampCursor;
declare const collapseDups: typeof import("./dedup.js").collapseDups;
declare const ageInDays: typeof import("./age.js").ageInDays;
