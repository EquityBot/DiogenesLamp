// The script tags in index.html expose these helpers to view-state.js.
// Reuse the CommonJS export types so both loading paths share one contract.
declare const normalizeFilters: typeof import("./filters.js").normalizeFilters;
declare const defaultFilters: typeof import("./filters.js").defaultFilters;
declare const matchSearch: typeof import("./filters.js").matchSearch;
declare const sortAlbums: typeof import("./filters.js").sortAlbums;
declare const clampCursor: typeof import("./catalogue.js").clampCursor;
declare const collapseDups: typeof import("./dedup.js").collapseDups;
declare const ageInDays: typeof import("./age.js").ageInDays;

declare const itemKeys: typeof import("./dedup.js").itemKeys;
declare const marketplaceItem: typeof import("./dedup.js").marketplaceItem;
declare const marketplaceUrl: typeof import("./dedup.js").marketplaceUrl;
declare const SHORTLINK_HOSTS: typeof import("./dedup.js").SHORTLINK_HOSTS;
declare const unwrapAgentLink: typeof import("./agents.js").unwrapAgentLink;
declare const agentLink: typeof import("./agents.js").agentLink;
declare const ACTIVE_AGENTS: typeof import("./agents.js").ACTIVE_AGENTS;
declare const readStored: typeof import("./filters.js").readStored;
declare const writeStored: typeof import("./filters.js").writeStored;
declare const compileCatalogueSearch: typeof import("./discovery.js").compileCatalogueSearch;
