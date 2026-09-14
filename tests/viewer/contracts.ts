// Compile only. Runtime behaviour is covered by the adjacent .test.cjs files.
import { defaultFilters, normalizeFilters } from "../../viewer/filters.js";
import { createViewState, nextViewState, selectCatalogue } from "../../viewer/view-state.js";

const view = createViewState(normalizeFilters({ hideDups: "invalid stored value" }));
nextViewState(view, { type: "filter", filters: defaultFilters() });
nextViewState(view, { type: "cursor", index: 1, count: 4 });
// @ts-expect-error Cursor positions must be numbers.
nextViewState(view, { type: "cursor", index: "1", count: 4 });
// @ts-expect-error Opening a group needs its identity and scroll position.
nextViewState(view, { type: "open-group" });
// @ts-expect-error Typed filter updates cannot store strings as booleans.
nextViewState(view, { type: "filter", filters: { ...defaultFilters(), hideDups: "yes" } });
// @ts-expect-error The reducer event name must match an implemented transition.
nextViewState(view, { type: "clear-filter" });

const selected = selectCatalogue(view, {
  albums: [], favorites: new Set<string>(), generatedAt: "", bannedKeys: new Set<string>(),
});
// @ts-expect-error Duplicate counts are numeric.
selected.dupCounts.set("group", "2");
