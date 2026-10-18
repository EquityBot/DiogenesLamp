"use strict";

/** @param {string} message */
function savedDataError(message) {
  const notice = document.getElementById("persistence-note");
  notice.textContent = message;
  notice.hidden = false;
}
const userStore = createUserStore(window, savedDataError);

/** @type {import("./controller-types.js").ViewerState} */
const state = {
  index: null,
  albums: [],          // all loaded albums, in shard order
  albumAliases: {},
  filtered: [],
  favorites: userStore.favorites(),
  collections: userStore.section("collection"),
  tags: userStore.section("tags"),
  reviews: userStore.section("review"),
  loadedSellers: 0,
  pendingShards: [],   // sellers whose shards are not yet fetched
  generatedAt: "",
  dupCounts: new Map(), // dup_key -> group size, valid for current filter pass
  dupHidden: 0,
  view: /** @type {import("./view-state.js").ViewState} */ (/** @type {unknown} */ (null)),         // catalogue choices, initialised after reading controls
  bannedKeys: new Set(), // corpus-global category-label keys, grown per shard
  titleKeyCounts: new Map(), // running t:-key counts feeding bannedKeys
  shardAlbums: new Map(), // seller id -> annotated albums, in fetch order
  failedShards: [],    // sellers whose shard could not be fetched
  categories: new Set(), // every category seen, collected as shards arrive
  filterVersion: 0,    // bumped whenever `filtered` is rebuilt
  prefs: readPrefs(window), // agent, lookups and currency choices (prefs.js)
};

const els = {
  priceMin: document.getElementById("price-min"),
  priceMax: document.getElementById("price-max"),
  includeUnknown: document.getElementById("include-unknown"),
  collection: document.getElementById("collection-filter"),
  tag: document.getElementById("tag-filter"),
  unreviewed: document.getElementById("unreviewed"),
  changed: document.getElementById("changed"),
  kind: document.getElementById("kind-filter"),
  descriptionCoverage: document.getElementById("description-coverage"),
  search: document.getElementById("search"),
  seller: document.getElementById("seller-filter"),
  category: document.getElementById("category-filter"),
  newOnly: document.getElementById("new-only"),
  favsOnly: document.getElementById("favs-only"),
  hideDups: document.getElementById("hide-dups"),
  age: document.getElementById("age-filter"),
  sort: document.getElementById("sort-filter"),
  exportFavs: document.getElementById("export-favs"),
  groupClear: document.getElementById("group-clear"),
  filtersClear: document.getElementById("filters-clear"),
  status: document.getElementById("status"),
  grid: document.getElementById("grid"),
  spacer: document.getElementById("grid-spacer"),
  container: document.getElementById("grid-container"),
  message: document.getElementById("grid-message"),
  progress: document.getElementById("load-progress-bar"),
  note: document.getElementById("load-note"),
  noteText: document.getElementById("load-note-text"),
  noteDismiss: document.getElementById("load-note-dismiss"),
  shopsPanel: document.getElementById("shops-panel"),
  shopsToggle: document.getElementById("shops-toggle"),
  themeToggle: document.getElementById("theme-toggle"),
  themeIcon: document.getElementById("theme-icon"),
};

// Current control values in the shape filters.js persists.
/** @returns {import("./filters.js").Filters} */
function currentFilters() {
  return {
    search: els.search.value,
    seller: els.seller.value,
    category: els.category.value,
    age: els.age.value,
    sort: els.sort.value,
    newOnly: els.newOnly.checked,
    favsOnly: els.favsOnly.checked,
    hideDups: els.hideDups.checked,
    priceMin: els.priceMin.value,
    priceMax: els.priceMax.value,
    includeUnknown: els.includeUnknown.checked,
    collection: els.collection.value,
    tag: els.tag.value,
    unreviewed: els.unreviewed.checked,
    changed: els.changed.checked,
    kind: els.kind.value,
  };
}

// Restoring a seller or category that the index no longer offers would leave
// the control blank while the filter still applied, so only values the
// dropdown actually holds are written back. The options for both arrive
// later than this runs, which is why applyStoredFilters is called again as
// they are added.
/** @param {HTMLSelectElement} select @param {string} value */
function setSelectIfPresent(select, value) {
  if (!value) return;
  if ([...select.options].some((option) => option.value === value)) {
    select.value = value;
  }
}

const storedFilters = filtersFromURL(location.search) || readFilters(window);

function applyStoredFilters() {
  els.search.value = storedFilters.search;
  els.age.value = storedFilters.age;
  els.sort.value = storedFilters.sort;
  els.newOnly.checked = storedFilters.newOnly;
  els.favsOnly.checked = storedFilters.favsOnly;
  els.hideDups.checked = storedFilters.hideDups;
  els.priceMin.value = storedFilters.priceMin;
  els.priceMax.value = storedFilters.priceMax;
  els.includeUnknown.checked = storedFilters.includeUnknown;
  els.unreviewed.checked = storedFilters.unreviewed;
  els.changed.checked = storedFilters.changed;
  for (const key of /** @type {const} */ (["collection", "tag"])) {
    if (storedFilters[key]) els[key].add(new Option(storedFilters[key], storedFilters[key]));
    els[key].value = storedFilters[key];
  }
  setSelectIfPresent(els.seller, storedFilters.seller);
  setSelectIfPresent(els.category, storedFilters.category);
  setSelectIfPresent(els.kind, storedFilters.kind);
}
// Kinds are a fixed table, so their options exist before any filter is restored.
for (const kind of KINDS) els.kind.add(new Option(kind.label, kind.id));
els.kind.add(new Option("Unsorted: no kind found in the title", UNSORTED));
applyStoredFilters();
state.view = createViewState(storedFilters, currentFilters());

// A figure in the tally, held at the width its largest value will need. The
// counts climb the whole time the catalogue streams in, and a number that
// widens as it grows would shift everything beside it on every shard.
/** @param {number} value @param {number} ceiling */
function tallyFigure(value, ceiling) {
  const span = document.createElement("span");
  span.className = "fig-n";
  span.textContent = value.toLocaleString();
  const width = Math.max(1, (ceiling || value).toLocaleString().length);
  span.style.minWidth = `${width}ch`;
  return span;
}

// Printed punctuation between fields, hidden from the live region: it is a
// rule on the page, and a screen reader reading it aloud is noise.
function tallySeparator() {
  const span = document.createElement("span");
  span.className = "sep";
  span.textContent = "·";
  span.setAttribute("aria-hidden", "true");
  return span;
}

function setStatus() {
  const total = state.index ? state.index.sellers.length : 0;
  const loading = !state.index || state.loadedSellers < total;
  // A seller whose scrape errored has no shard on disk, so its fetch 404s.
  // Those still count in loadedSellers (the attempt finished), so name the
  // missing part rather than silently reporting a full catalogue.
  const parts = [
    tallyFigure(state.filtered.length, state.albums.length),
    state.filtered.length === 1 ? " album" : " albums",
    tallySeparator(), " ",
    tallyFigure(state.loadedSellers, total),
    `/${total} ${total === 1 ? "seller" : "sellers"}`,
  ];
  if (state.failedShards.length) {
    parts.push(tallySeparator(), " ",
      `${state.failedShards.length} unavailable`);
  }
  if (state.dupHidden) {
    parts.push(tallySeparator(), " ",
      `${state.dupHidden.toLocaleString()} duplicates hidden`);
  }
  els.status.replaceChildren(...parts);

  els.progress.style.transform =
    `scaleX(${total ? state.loadedSellers / total : 0})`;
  els.progress.parentElement.hidden = !loading;
  // Announcing an incomplete grid as ready would tell a screen reader to
  // read a list that is still growing under it.
  els.grid.setAttribute("aria-busy", String(loading));
  els.filtersClear.hidden = !anyFilterActive(currentFilters()) || Boolean(state.view.groupKey);
  setGridMessage(loading);
  setLoadNote(loading);
}

// The blank grid is the single most confusing state in the viewer: it looks
// identical whether nothing has loaded yet, nothing matched, or every match
// was collapsed as a duplicate. Name which one it is.
/** @param {boolean} loading */
function setGridMessage(loading) {
  if (state.filtered.length) {
    els.message.hidden = true;
    return;
  }
  let text;
  if (loading) {
    text = "Loading the catalog...";
  } else if (!state.albums.length) {
    // Distinguishing "collapsed as duplicates" here would be dead code:
    // collapseDups keeps one card per group, so an empty result can only
    // come from an empty input, which means dupHidden is zero too.
    text = "No albums in this catalog yet. Run `lamp scrape` to fill it.";
  } else {
    text = "No albums match these filters. Nothing interesting happens. "
      + "Clear them to see the whole catalogue again.";
  }
  els.message.textContent = text;
  els.message.hidden = false;
}

// Failed shards used to live in a title attribute, which a keyboard or touch
// user never sees. Surface them once the load settles, and let them be
// dismissed rather than sitting there for the session.
let noteDismissed = false;
/** @param {boolean} loading */
function setLoadNote(loading) {
  const failed = state.failedShards.length;
  if (loading || !failed || noteDismissed) {
    els.note.hidden = true;
    return;
  }
  const names = state.failedShards.join(", ");
  els.noteText.textContent =
    `${failed} ${failed === 1 ? "seller's data is" : "sellers' data is"} missing or ` +
    `unreadable and ${failed === 1 ? "was" : "were"} left out: ${names}.`;
  els.note.hidden = false;
}

const SHARD_CONCURRENCY = 6;

async function loadAll() {
  /** @type {import("./controller-types.js").CatalogueIndex} */
  let index;
  try {
    const response = await fetch("data/index.json");
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    index = await response.json();
    if (!validCatalogueIndex(index)) throw new Error("Invalid catalogue index");
  } catch (err) {
    // Without the manifest nothing below can run; name the cause on screen
    // instead of leaving the "Loading the catalog..." message up forever.
    console.error("index.json failed to load", err);
    els.message.textContent =
      "Could not load data/index.json - run `lamp scrape` first, and serve " +
      "the viewer from the directory that holds data/.";
    els.message.hidden = false;
    els.progress.parentElement.hidden = true;
    els.grid.setAttribute("aria-busy", "false");
    return;
  }
  state.index = index;
  state.albumAliases = {};
  refreshSavedState(false);
  state.generatedAt = index.generated_at;
  for (const seller of index.sellers) {
    const opt = document.createElement("option");
    opt.value = seller.id;
    opt.textContent = `${seller.name} (${seller.album_count})`;
    els.seller.appendChild(opt);
  }
  renderShops(index.shops);
  // The seller list only exists now, so a stored seller filter can finally
  // be put back on the control.
  restoreLateFilters();
  state.pendingShards = [...index.sellers];
  await Promise.all(
    Array.from({ length: SHARD_CONCURRENCY }, () => shardWorker())
  );
  refreshUI(); // final flush so the last shards always land on screen
}

// Spec: filtering to one seller loads that shard first. Re-check the
// dropdown before every fetch so a mid-load selection jumps the queue.
function nextShard() {
  const selected = els.seller.value;
  let nextIdx = 0;
  if (selected) {
    const hit = state.pendingShards.findIndex((s) => s.id === selected);
    if (hit >= 0) nextIdx = hit;
  }
  return state.pendingShards.splice(nextIdx, 1)[0];
}

async function shardWorker() {
  while (state.pendingShards.length) {
    const seller = nextShard();
    if (!seller) return;
    try {
      // fetch resolves on 404 too, so check the status: without this the
      // only thing failing is JSON.parse on an error page, which reports a
      // syntax error instead of a missing seller.
      const response = await fetch(`data/${seller.shard}`);
      if (!response.ok) throw new Error(`HTTP ${response.status}`);
      const shard = /** @type {import("./controller-types.js").CatalogueShard} */ (await response.json());
      const admitted = admittedCatalogueShard(shard, seller);
      if (!admitted) throw new Error("Invalid or incomplete selected catalogue shard");
      shard.albums = admitted;
      let annotated = 0;
      for (const album of shard.albums) {
        album.dup_key = dupKey(album);
        album.kind = kindOf(album.title);
        prepareAlbumSearch(album);
        album.published_ms = publishedMs(album);
        // ponytail: yield every 1000 albums; use a time budget if slow devices still stall.
        if (++annotated % 1000 === 0) {
          await new Promise(resolve => setTimeout(() => resolve(null), 0));
        }
      }
      markNew(shard.albums, state.generatedAt);
      // Bans are a corpus property: fold this batch into the running
      // counts, never derive them from a filtered view.
      registerTitleKeys(shard.albums, state.titleKeyCounts, state.bannedKeys);
      // Only a freshly loaded shard can hold a category not seen yet, so
      // collect them here instead of re-walking the whole corpus on every
      // refresh while it streams in.
      for (const album of shard.albums) {
        if (album.category) state.categories.add(album.category);
      }
      state.shardAlbums.set(seller.id, shard.albums);
    } catch (err) {
      console.warn(`shard failed: ${seller.id}`, err);
      state.failedShards.push(seller.id);
    }
    state.loadedSellers += 1;
    scheduleRefresh();
  }
}

// Concurrent fetches complete in arbitrary order; rebuilding state.albums
// from the index's seller order keeps the browse order deterministic
// across reloads instead of varying with network timing.
function rebuildAlbums() {
  const loaded = /** @type {import("./controller-types.js").CatalogueIndex} */ (state.index)
    .sellers.flatMap(seller => state.shardAlbums.get(seller.id) || []);
  const selected = newestListings(loaded);
  state.albums = selected.albums;
  state.albumAliases = inheritableAliases(state.index || {}, state.albums, selected.aliases,
    state.loadedSellers === state.index?.sellers.length && !state.failedShards?.length);
}

// Descriptions arrive as albums are refetched, so for a long while the
// search covers some albums' text and not others'. Say so, in numbers,
// against the loaded corpus.
function setDescriptionCoverage() {
  let recorded = 0, empty = 0, excerpts = 0;
  for (const album of state.albums) {
    if (!("description" in album)) continue;
    recorded += 1;
    if (!album.description) empty += 1;
    if (album.description_truncated) excerpts += 1;
  }
  const total = state.albums.length;
  els.descriptionCoverage.textContent = !total ? "" :
    `Descriptions recorded for ${recorded} of ${total} loaded albums` +
    (empty ? `, ${empty} with no text extracted` : "") +
    (excerpts ? `, ${excerpts} saved as excerpts` : "") +
    `; ${total - recorded} not recorded yet. Search covers saved text only.`;
}

// With several shard fetches landing concurrently, refreshing the grid per
// shard would filter and re-render the whole corpus dozens of times a
// second. Coalesce to at most one refresh per 200ms.
/** @type {number | ReturnType<typeof setTimeout>} */
let refreshTimer = 0;
function scheduleRefresh() {
  if (refreshTimer) return;
  refreshTimer = setTimeout(refreshUI, 200);
}

function refreshUI() {
  clearTimeout(refreshTimer);
  refreshTimer = 0;
  // Streaming loads call this several times a second while the user may be
  // deep in the grid. Inserting a shard above the viewport would otherwise
  // yank every card down: anchor on the topmost visible album and restore
  // its pixel position after the rebuild instead.
  const anchor = topVisibleAlbum();
  rebuildAlbums();
  refreshSavedState(false);
  setDescriptionCoverage();
  refreshCategoryOptions();
  applyFilters();
  restoreAnchor(anchor);
}

// The album at the top of the viewport before a background rebuild, with
// its position. null when nothing is on screen yet (still loading the
// first shards, or the list is empty). The arithmetic lives in anchor.js
// so it can be tested without a DOM; this only measures.
function topVisibleAlbum() {
  const spot = anchorAt(
    window.scrollY - els.spacer.offsetTop,
    rowHeight(),
    columns(),
    state.filtered.length
  );
  if (!spot) return null;
  const album = state.filtered[spot.index];
  return album ? { key: cardKey(album), offset: spot.offset } : null;
}

// Put the anchored album back at its pixel offset. No-op when the album is
// gone (filtered out by a late category restore) or nothing moved.
/** @param {{key: string, offset: number} | null} anchor */
function restoreAnchor(anchor) {
  if (!anchor) return;
  const idx = state.filtered.findIndex((a) => cardKey(a) === anchor.key);
  if (idx < 0) return;
  const viewTop = restoredViewTop(idx, anchor.offset, rowHeight(), columns());
  window.scrollTo(0, viewTop + els.spacer.offsetTop);
}

function refreshCategoryOptions() {
  const known = new Set([...els.category.options].map((option) => option.value));
  let added = false;
  for (const category of state.categories) {
    if (known.has(category)) continue;
    known.add(category);
    added = true;
    const opt = document.createElement("option");
    opt.value = opt.textContent = category;
    els.category.appendChild(opt);
  }
  // A stored category filter can only be restored once its option exists,
  // which is whenever the shard carrying that category lands.
  if (added) restoreLateFilters();
}

// The query the visible captions are marked against. Set once per pass
// rather than read from the control per card: a filtered render builds a few
// hundred captions and they all belong to the same query.
let markQuery = "";

function applyFilters() {
  markQuery = state.view.filters.search.trim();
  const invalidPrice = !els.priceMin.validity.valid || !els.priceMax.validity.valid || Boolean(els.priceMin.value && els.priceMax.value &&
    Number(els.priceMin.value) > Number(els.priceMax.value));
  document.getElementById("price-error").hidden = !invalidPrice;
  els.priceMax.setAttribute("aria-invalid", String(invalidPrice));
  setGroupGeometry();
  const selected = invalidPrice ? {filtered: [], dupCounts: new Map(), dupHidden: 0, cursor: 0}
    : selectCatalogue(state.view, state);
  state.filtered = selected.filtered;
  state.dupCounts = selected.dupCounts;
  state.dupHidden = selected.dupHidden;
  state.filterVersion += 1;
  setCursor(selected.cursor);
  els.groupClear.hidden = !state.view.groupKey;
  if (state.view.groupKey) {
    const sellers = new Set(state.filtered.map(a => a.seller)).size;
    els.groupClear.replaceChildren(
      drawnMark("mark-cross"),
      ` ${state.filtered.length} copies from ` +
      `${sellers} ${sellers === 1 ? "seller" : "sellers"}`
    );
  }
  setStatus();
  // Reuse the existing card window during streaming catalogue refreshes.
  renderGrid();
}

// Entering or leaving the strip page changes the card pitch, and app.js reads
// that pitch out of the stylesheet. Re-read it here, before anything is placed
// on it, or the first render after the switch lays the new windows out on the
// old geometry and overlaps every row.
let groupViewOn = false;
function setGroupGeometry() {
  const wanted = Boolean(state.view.groupKey);
  if (wanted === groupViewOn) return;
  groupViewOn = wanted;
  document.documentElement.classList.toggle("group-view", wanted);
  geometry = readGeometry();
  // Every card in the window belongs to the other page: the strip carries a
  // date the lattice does not, and a reused node would arrive without one.
  els.grid.replaceChildren();
  cardCache = new Map();
  lastRange = { start: -1, end: -1, cols: -1, count: -1, version: -1 };
}

// A posted date, for the strip page: the whole question a group answers is
// which seller posted the same thing and when.
/** @param {import("./controller-types.js").ViewerAlbum} album */
function postedOn(album) {
  if (!album.published_ms) return "no date";
  return new Date(album.published_ms).toISOString().slice(0, 10);
}

// Text content only: the DOM escapes &, < and > but leaves quotes alone,
// which is fine between tags and unsafe inside an attribute.
/** @param {string} text */
function escapeHtml(text) {
  const div = document.createElement("div");
  div.textContent = text;
  return div.innerHTML;
}

// Attribute values need the quotes escaped too, or a value containing one
// closes the attribute early and everything after it is parsed as markup.
/** @param {string | null | undefined} value */
function escapeAttr(value) {
  return escapeHtml(value ?? "").replace(/"/g, "&quot;").replace(/'/g, "&#39;");
}

// Stable hue per album, so a cover-less card keeps the same colour across
// reloads and neighbouring cards rarely match. Any cheap string hash does;
// this is FNV-1a folded onto the colour wheel.
/** @param {string} seed */
function coverHue(seed) {
  let hash = 2166136261;
  for (let i = 0; i < seed.length; i += 1) {
    hash ^= seed.charCodeAt(i);
    hash = Math.imul(hash, 16777619);
  }
  return Math.abs(hash) % 360;
}

/** @param {import("./controller-types.js").ViewerAlbum} album */
function thumbVersion(album) {
  return encodeURIComponent(album.thumb_version || album.date_modified || album.updated_at || "");
}

const SVG_NS = "http://www.w3.org/2000/svg";

// One drawn mark from the sheet in index.html. Built rather than written as
// markup so the same mark set serves the cards, the buttons and the labels
// without 3 copies of the same path data.
/** @param {string} id */
function drawnMark(id, className = "mark") {
  const svg = document.createElementNS(SVG_NS, "svg");
  svg.setAttribute("class", className);
  svg.setAttribute("aria-hidden", "true");
  svg.setAttribute("focusable", "false");
  const use = document.createElementNS(SVG_NS, "use");
  use.setAttribute("href", `#${id}`);
  svg.appendChild(use);
  return svg;
}

// The title, with the spans the current query hit struck in the second ink.
// Built from text nodes rather than interpolated markup: the title is scraped
// remote HTML, and the offsets here would be the one place an escape could go
// missing.
/** @param {HTMLElement} el @param {string} title @param {string} query */
function fillTitle(el, title, query) {
  const ranges = matchRanges(title, query);
  if (!ranges.length) {
    el.textContent = title;
    return;
  }
  const nodes = [];
  let at = 0;
  for (const [start, end] of ranges) {
    if (start > at) nodes.push(title.slice(at, start));
    const hit = document.createElement("mark");
    hit.textContent = title.slice(start, end);
    nodes.push(hit);
    at = end;
  }
  if (at < title.length) nodes.push(title.slice(at));
  el.replaceChildren(...nodes);
}

// A card outlives the query that built it: the render loop reuses cards when
// the filter changes, so the marks drawn for the previous query stayed on the
// title. Refill only when the query actually changed, so scrolling still
// leaves the cards it reuses alone.
/** @param {HTMLDivElement} card @param {import("./controller-types.js").ViewerAlbum} album @param {string} query */
function refreshCardTitle(card, album, query) {
  if (card.dataset.markQuery === query) return false;
  fillTitle(/** @type {HTMLElement} */ (card.querySelector(".card-title span")), album.title, query);
  card.dataset.markQuery = query;
  return true;
}

/** @param {import("./controller-types.js").ViewerAlbum} album */
function makeCard(album) {
  const card = document.createElement("div");
  card.className = "card";
  const isNew = album.is_new;
  if (isNew) card.classList.add("is-new");
  const isFav = state.favorites.has(album.id);
  // Everything interpolated below reaches us from scraped remote HTML. The
  // scraper constrains platform to a known set and builds URLs itself, so
  // nothing hostile is expected - but escaping is a line of code and the
  // alternative is trusting that forever.
  // With an agent chosen the mark keeps its platform name and opens on the
  // agent, rebuilt from the item id; the marketplace link stays in the preview.
  const agent = ACTIVE_AGENTS.find((candidate) => candidate.id === state.prefs.agent);
  const links = album.links.map((link) => {
    const via = agent ? agentLink(agent.id, marketplaceItem(link.url)) : "";
    const opens = via && agent ? `${escapeAttr(link.platform)}, opens on ${escapeAttr(agent.name)}` : "";
    return `<a class="${escapeAttr(link.platform)}" href="${escapeAttr(via || link.url)}" ` +
      (opens ? `title="${opens}" aria-label="${opens}" ` : "") +
      `target="_blank" rel="noreferrer">${escapeHtml(link.platform)}</a>`;
  }).join("");
  const dupCount = album.dup_key ? state.dupCounts.get(album.dup_key) || 0 : 0;
  // Remember what the badge was built from: a later pass can reuse this
  // node only while that count still holds.
  card.dataset.dupCount = String(dupCount);
  card.setAttribute("role", "listitem");
  card.innerHTML =
    // The titled colour block sits behind the cover and shows through when
    // the image fails or is absent - some albums have no cover on Yupoo at
    // all. A loaded cover simply paints over it, so nothing needs to know
    // which case it is.
    // aria-hidden: this repeats .card-title purely as a visual fallback for
    // a missing cover, and without it every card is announced twice.
    `<div class="card-cover">` +
    `<span class="card-cover-title" aria-hidden="true">${escapeHtml(album.title)}</span>` +
    // ?v busts the day-long thumb cache when the cover can have changed.
    // Prefer Yupoo's own dateModified: our updated_at is stamped on every
    // re-fetch, so keying on it would re-download every thumb after a
    // scrape that changed nothing.
    // No thumb means Yupoo had no cover for the album. Emitting an img with
    // an empty path would request the data directory itself and take a 404
    // per card; the colour block behind it is already the intended fallback.
    (album.thumb
      ? `<img class="card-thumb" loading="lazy" alt="" ` +
        `src="data/${escapeAttr(album.thumb)}?v=${thumbVersion(album)}">`
      : "") +
    // The window is the way into the album. A caption chip saying so was one
    // more thing in a row with no room for it, and the picture is what anyone
    // reaches for anyway. Empty and labelled: the title is already on the
    // card, and repeating it here would announce every album twice.
    `<a class="cover-link" href="${escapeAttr(album.album_url)}" target="_blank" ` +
    `rel="noreferrer" aria-label="Open album: ${escapeAttr(album.title)}"></a>` +
    // The marks live inside the cover so they are positioned against it
    // rather than against a hard-coded offset that has to be kept in step
    // with the cover height at every breakpoint. They sit above the link so
    // it cannot swallow their clicks.
    (isNew ? `<span class="tag-new">NEW</span>` : "") +
    (album.price ? `<span class="badge-price">${escapeHtml(displayPrice(album.price, state.prefs, formatPrice))}</span>` : "") +
    (dupCount > 1
      ? `<button class="badge-dup" type="button" ` +
        `aria-label="Show all ${dupCount} copies of this posting">` +
        `&times;${dupCount}</button>`
      : "") +
    `<button class="fav-btn" type="button" aria-pressed="${isFav}" ` +
    `aria-label="${favLabel(isFav)}"></button>` +
    `</div>` +
    `<button class="card-title preview-btn" type="button" aria-label="Preview: ${escapeAttr(album.title)}"><span></span></button>` +
    `<div class="card-meta">` +
    `<span class="seller-tag">${escapeHtml(album.seller)}</span>` +
    `<span class="card-links">${links}</span>` +
    // On the strip page the copies differ by seller and date and nothing
    // else, so the date earns its place there and only there.
    (state.view.groupKey ? `<span class="posted">${escapeHtml(postedOn(album))}</span>` : "") +
    `</div>`;
  // The stylesheet reads the hue from a custom property, and it is set here
  // rather than in the markup above: the page is served under a policy with
  // no unsafe-inline, so a style attribute written into parsed HTML never
  // applies. Setting the property through the CSSOM is not inline style and
  // is unaffected.
  /** @type {HTMLElement} */ (card.firstElementChild).style.setProperty("--cover-hue", String(coverHue(album.id)));
  /** @type {HTMLButtonElement} */ (card.querySelector(".fav-btn")).appendChild(drawnMark("mark-mount"));
  refreshCardTitle(card, album, markQuery);
  // An inline onerror attribute did this before. Binding it here keeps the
  // markup free of executable attributes, which is what lets a strict
  // script-src policy apply to this page unchanged.
  const thumb = card.querySelector(".card-thumb");
  if (thumb) {
    thumb.addEventListener("error", (event) => {
      /** @type {HTMLElement} */ (event.currentTarget).style.display = "none";
    });
  }
  /** @type {HTMLButtonElement} */ (card.querySelector(".fav-btn")).addEventListener("click", (event) => {
    toggleFavorite(album.id, /** @type {Element} */ (event.currentTarget));
  });
  /** @type {HTMLButtonElement} */ (card.querySelector(".preview-btn")).addEventListener("click", () => openPreview(album.id));
  const dupBtn = card.querySelector(".badge-dup");
  if (dupBtn) {
    dupBtn.addEventListener("click", () => {
      state.view = nextViewState(state.view, {
        type: "open-group", groupKey: /** @type {string} */ (album.dup_key), scrollY: window.scrollY,
      });
      window.scrollTo(0, 0);
      applyFilters();
      // the badge just got re-rendered away - park focus on the exit chip
      els.groupClear.focus({ preventScroll: true });
    });
  }
  return card;
}

// The star alone carries no state for a screen reader, so the button owns
// both a pressed state and a name that says what pressing it will do.
/** @param {boolean} isFav */
function favLabel(isFav) {
  return isFav ? "Remove from favourites" : "Add to favourites";
}

/** @param {string} albumId @param {Element | null} button */
function toggleFavorite(albumId, button) {
  const wanted = userStore.get("favorite", albumId) !== true;
  if (!userStore.set("favorite", albumId, wanted)) return;
  refreshSavedState();
  if (button?.isConnected) {
    button.setAttribute("aria-pressed", String(wanted));
    button.setAttribute("aria-label", favLabel(wanted));
  }
}

// Card geometry lives in style.css and is read from there. Duplicating the
// numbers here would let a CSS tweak silently desynchronise the absolute
// positions this file computes, leaving overlapping or gapped rows.
// Re-read rather than cached once: the card size changes at the narrow
// breakpoint, and a stale copy would place every card on the old pitch,
// leaving overlapping rows on a phone or after a window resize.
function readGeometry() {
  const css = getComputedStyle(document.documentElement);
  /** @param {string} name @param {number} fallback */
  const px = (name, fallback) => Number.parseInt(css.getPropertyValue(name), 10) || fallback;
  const gap = px("--gap", 12);
  return { w: px("--card-w", 220) + gap, h: px("--card-h", 300) + gap, gap };
}
let geometry = readGeometry();

function columns() {
  const width = els.container.clientWidth - geometry.gap * 2;
  // The last column carries no trailing gap, which is how the spacer is ruled.
  // Charging one here dropped a column whenever the leftover reached a card width.
  return Math.max(1, Math.floor((width + geometry.gap) / geometry.w));
}
function rowHeight() { return geometry.h; }
/** @param {number} count */
function rowsFor(count) { return Math.ceil(count / columns()); }
// cols is passed in, not read here: the render loop writes card.style.transform
// and columns() reads container.clientWidth, so reading it per card forced a
// synchronous layout on every card of every scroll frame.
/** @param {number} i @param {number} cols */
function placeCard(i, cols) {
  const x = (i % cols) * geometry.w;
  const y = Math.floor(i / cols) * geometry.h;
  return `translate(${x}px, ${y}px)`;
}

// Windowed rendering: only rows near the viewport exist in the DOM.
const OVERSCAN_ROWS = 3;
/** @type {{start: number, end: number, cols: number, count: number, version: number, cursor?: number}} */
let lastRange = { start: -1, end: -1, cols: -1, count: -1, version: -1, cursor: -1 };
// Cards currently in the grid. Scroll-driven renders keep them (same nodes,
// listeners, decoded thumbs) and touch only the rows entering and leaving
// the window; anything that can change card content or membership goes
// through renderGrid(force = true), which rebuilds from scratch.
// Keyed by seller and id together: an id repeated in one view would
// otherwise claim the same node twice and strand a card in the DOM.
// The separator is written as an escape, the way links.rs writes its own
// control characters: a literal NUL in a source file is invisible to the
// next reader and makes the file binary to grep and ripgrep, which then
// report "binary file matches" and print no lines at all.
/** @type {Map<string, HTMLDivElement>} */
let cardCache = new Map();
/** @param {import("./controller-types.js").ViewerAlbum} album */
const cardKey = (album) => `${album.seller}\u0000${album.id}`;

function renderGrid(force = false) {
  const cols = columns();
  const rowH = rowHeight();
  const totalRows = rowsFor(state.filtered.length);
  els.spacer.style.height = `${totalRows * rowH}px`;
  // The leaf is exactly as wide as the columns it holds. Left at full width it
  // rules ground no window will ever sit on, which reads as a column that
  // failed to load rather than as a margin.
  const ruledCols = Math.max(1, Math.min(cols, state.filtered.length));
  els.spacer.style.width = `${ruledCols * geometry.w - geometry.gap}px`;

  // Measured from the spacer, not its container: the load note and the empty
  // message sit above it inside the same container, so anchoring on the
  // container would offset every card by whatever those occupy.
  const viewTop = window.scrollY - els.spacer.offsetTop;
  const firstRow = Math.max(0, Math.floor(viewTop / rowH) - OVERSCAN_ROWS);
  const lastRow = Math.min(
    totalRows,
    Math.ceil((viewTop + window.innerHeight) / rowH) + OVERSCAN_ROWS
  );
  // filterVersion is part of the key: 2 different filters can produce the
  // same row range and the same count, and without it the second would be
  // mistaken for a no-op and never drawn. So is the cursor, since the pass
  // below is what stamps the tab stop - sameRenderWindow says why.
  const wanted = { start: firstRow, end: lastRow, cols,
                   count: state.filtered.length, version: state.filterVersion,
                   cursor: state.view.cursor };
  if (!force && sameRenderWindow(lastRange, wanted)) return;
  lastRange = wanted;

  if (force) {
    els.grid.replaceChildren();
    cardCache = new Map();
  }
  // Apply only the difference. Re-appending every card each scroll step
  // re-parented ~150 nodes to move by one row; now the cards that stay put
  // are left alone and only the entering and leaving rows are touched.
  /** @type {Map<string, HTMLDivElement>} */
  const next = new Map();
  const startIdx = firstRow * cols;
  const endIdx = Math.min(state.filtered.length, lastRow * cols);
  for (let i = startIdx; i < endIdx; i += 1) {
    const album = state.filtered[i];
    const key = cardKey(album);
    /** @type {HTMLDivElement | null | undefined} */
    let card = cardCache.get(key);
    if (card) {
      cardCache.delete(key);  // claimed - whatever is left over has scrolled off
      // The badge is the only part of a card that depends on the filter
      // pass rather than the album, so a changed count means rebuild.
      const dupCount = album.dup_key ? state.dupCounts.get(album.dup_key) || 0 : 0;
      if (card.dataset.dupCount !== String(dupCount)) {
        card.remove();
        card = null;
      }
      // The title's marks belong to the query that drew them, and the query
      // can change without the card being rebuilt.
      if (card) refreshCardTitle(card, album, markQuery);
    }
    if (!card) {
      card = makeCard(album);
      els.grid.appendChild(card);
    }
    const fav = /** @type {HTMLButtonElement} */ (card.querySelector(".fav-btn"));
    setIfChanged(fav, "aria-pressed", String(state.favorites.has(album.id)));
    setIfChanged(fav, "aria-label", favLabel(state.favorites.has(album.id)));
    const transform = placeCard(i, cols);
    if (card.style.transform !== transform) card.style.transform = transform;
    // Only a window of rows exists in the DOM, so role=list would otherwise
    // advertise a set the size of that window: "item 1 of 30" against a
    // catalogue of 6 figures. Stamped here rather than in makeCard because a
    // cached card is reused at a different index when the filter changes,
    // and only this loop knows the new one.
    setIfChanged(card, "aria-setsize", String(state.filtered.length));
    setIfChanged(card, "aria-posinset", String(i + 1));
    // Roving tabindex: one stop for the whole lattice, and the arrows move
    // it. Forty thousand tab stops is not a keyboard interface.
    setCardTabStop(card, i === state.view.cursor);
    next.set(key, card);
  }

  // Remove departed cards before reordering, so the order pass never sees a
  // node that is on its way out. A card holding focus is not simply dropped:
  // that leaves the keyboard user on <body> with their place lost, which
  // happens both when they scroll a focused card out of the window and when
  // un-favouriting a card removes it from a favourites-only view.
  const departed = [...cardCache.values()];
  // Published before rescueFocus, not after: the focus it hands out fires
  // focusin, which moves the cursor and restamps the tab stop across the
  // window. Left until the end of the function, that restamp would run over
  // the window this pass is replacing and leave the stop on a card already
  // on its way out. `departed` is a snapshot, so nothing below needs the old
  // map back.
  cardCache = next;
  rescueFocus(departed, next);
  for (const node of departed) node.remove();

  // DOM order has to match visual order or Tab walks the grid in the order
  // rows happened to be appended. Scrolling up appends the entering rows
  // after the surviving ones, so tabbing threw the viewport backwards by
  // whole screens. Surviving cards are already in relative order, so this
  // moves only the cards that actually entered.
  /** @type {HTMLDivElement | null} */
  let previous = null;
  for (const card of next.values()) {
    if (card.previousElementSibling !== previous) {
      els.grid.insertBefore(card, previous ? previous.nextSibling : els.grid.firstChild);
    }
    previous = card;
  }
}

/** @param {Element} el @param {string} name @param {string} value */
function setIfChanged(el, name, value) {
  if (el.getAttribute(name) !== value) el.setAttribute(name, value);
}

// Everything on a card that the browser makes focusable in its own right.
// Stamping the tabindex on the card alone leaves all of these in the tab
// order, and a card carries a cover link, a star, sometimes a duplicate
// badge and up to 3 purchase links: at ~54 cards in the render window
// that is several hundred stops to tab past the grid, which is the outcome
// the roving tabindex exists to prevent. A card added to makeCard with a
// new focusable element has to be added here too.
const CARD_STOPS = ".cover-link, .fav-btn, .badge-dup, .card-links a, .preview-btn";

// The card under the cursor is the stop, and its own controls follow it, so
// they stay reachable by Tab without every other card offering them too.
/** @param {HTMLDivElement} card @param {boolean} isCursor */
function setCardTabStop(card, isCursor) {
  const value = isCursor ? "0" : "-1";
  setIfChanged(card, "tabindex", value);
  for (const el of card.querySelectorAll(CARD_STOPS)) {
    setIfChanged(el, "tabindex", value);
  }
}

// Cursor transitions also update the tab stop. The stop has to move with it, and
// a render cannot be relied on to do that: focusin - a click on a card, or a
// Tab into one - moves the cursor and renders nothing at all. So the window
// is restamped here and now.
//
// Restamped across the whole window rather than handed from one card to
// another, because applyFilters rebuilds state.filtered before it clamps the
// cursor, so the outgoing index no longer names the outgoing card. The window
// holds a few dozen cards and setIfChanged writes only what actually differs,
// which is what the render loop does on every pass anyway.
/** @param {number} next */
function setCursor(next) {
  state.view = nextViewState(state.view, {
    type: "cursor", index: next, count: state.filtered.length,
  });
  const album = state.filtered[state.view.cursor];
  const key = album ? cardKey(album) : null;
  for (const [id, card] of cardCache) setCardTabStop(card, id === key);
}

// Focus follows the user, so hand it to the same control on the card that
// took the removed one's place; failing that, park it on the grid itself,
// which is focusable for the skip link anyway. preventScroll throughout:
// this runs mid-render and must not fight the scroll that triggered it.
/** @param {HTMLDivElement[]} departed @param {Map<string, HTMLDivElement>} survivors */
function rescueFocus(departed, survivors) {
  const active = document.activeElement;
  if (!active || !departed.some((node) => node.contains(active))) return;
  const selector = active.classList.contains("fav-btn") ? ".fav-btn"
    : active.classList.contains("badge-dup") ? ".badge-dup"
      : active.classList.contains("cover-link") ? ".cover-link" : null;
  if (selector) {
    for (const card of survivors.values()) {
      const target = /** @type {HTMLElement | null} */ (card.querySelector(selector));
      if (target) {
        target.focus({ preventScroll: true });
        return;
      }
    }
  }
  els.grid.focus({ preventScroll: true });
}

// Keyboard movement across the lattice. Only a window of rows is in the DOM,
// so a move goes in one order and no other: pick the index, put its row on
// screen, render, then take focus. Focusing first lands on a node the render
// is about to recycle away.
const GRID_KEYS = new Set(["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown",
  "Home", "End", "PageUp", "PageDown"]);

// A Page move travels a screenful. The arithmetic lives in catalogue.js so
// it can be tested without a DOM; this only measures.
function albumsPerPage() {
  return pageStep(window.innerHeight, rowHeight(), columns());
}

// How much of the top of the page the header is covering. Nothing but the
// gutter on narrow screens, where it scrolls away with the content instead
// of sticking.
function headroom() {
  return getComputedStyle(topbar).position === "sticky"
    ? topbar.offsetHeight + geometry.gap : geometry.gap;
}

/** @param {number} next */
function moveCursor(next) {
  if (!state.filtered.length) return;
  setCursor(next);
  const rowH = rowHeight();
  const rowTop = els.spacer.offsetTop + Math.floor(state.view.cursor / columns()) * rowH;
  const above = window.scrollY + headroom();
  const below = window.scrollY + window.innerHeight;
  if (rowTop < above) window.scrollTo(0, rowTop - headroom());
  else if (rowTop + rowH > below) window.scrollTo(0, rowTop + rowH - window.innerHeight);
  renderGrid();
  const card = cardCache.get(cardKey(state.filtered[state.view.cursor]));
  if (card) card.focus({ preventScroll: true });
}

// Only when a window itself holds focus: inside a link or a button the same
// keys belong to that control.
function focusedCard() {
  const active = document.activeElement;
  return active && active.classList.contains("card") ? active : null;
}

els.grid.addEventListener("keydown", (event) => {
  // Give browser shortcuts such as Ctrl/Cmd+F, Ctrl/Cmd+P and Alt+arrows priority.
  // Plain grid keys and their Shift variants retain the catalogue actions below.
  if (event.ctrlKey || event.metaKey || event.altKey) return;
  const card = focusedCard();
  if (!card) return;
  if (GRID_KEYS.has(event.key)) {
    event.preventDefault();
    moveCursor(gridStep(state.view.cursor, event.key, columns(),
      state.filtered.length, albumsPerPage()));
    return;
  }
  const album = state.filtered[state.view.cursor];
  if (!album) return;
  if (event.key === "Enter" && /** @type {Element} */ (event.target).closest("button, a")) return;
  if (event.key === "p" || event.key === "P") {
    event.preventDefault();
    openPreview(album.id);
  } else if (event.key === "Enter") {
    event.preventDefault();
    window.open(album.album_url, "_blank", "noreferrer");
  } else if (event.key === "f" || event.key === "F") {
    event.preventDefault();
    toggleFavorite(album.id, card.querySelector(".fav-btn"));
  }
});

// Tabbing into a window makes it the cursor, so the arrows carry on from
// wherever the user actually is rather than from where they last were.
els.grid.addEventListener("focusin", (event) => {
  const card = /** @type {Element} */ (event.target).closest(".card");
  if (!card) return;
  const at = Number(card.getAttribute("aria-posinset")) - 1;
  if (Number.isInteger(at) && at >= 0) setCursor(at);
});

let scrollScheduled = false;
window.addEventListener("scroll", () => {
  if (scrollScheduled) return;
  scrollScheduled = true;
  requestAnimationFrame(() => {
    scrollScheduled = false;
    renderGrid();
  });
}, { passive: true });

window.addEventListener("resize", () => {
  geometry = readGeometry();
  renderGrid(true);
});

/** @param {import("./controller-types.js").Shop[]} shops */
function renderShops(shops) {
  /** @type {Map<string, import("./controller-types.js").Shop[]>} */
  const groups = new Map();
  for (const shop of shops) {
    if (!groups.has(shop.platform)) groups.set(shop.platform, []);
    /** @type {import("./controller-types.js").Shop[]} */ (groups.get(shop.platform)).push(shop);
  }
  els.shopsPanel.replaceChildren();
  // A slide-over that covers the grid needs a way out that is not the
  // toggle behind it, for touch users and for anyone tabbing into the panel.
  const close = document.createElement("button");
  close.type = "button";
  close.id = "shops-close";
  close.textContent = "Close shops";
  close.addEventListener("click", () => setShopsOpen(false));
  els.shopsPanel.appendChild(close);
  for (const [platform, entries] of [...groups.entries()].sort()) {
    const heading = document.createElement("h3");
    heading.textContent = platform;
    els.shopsPanel.appendChild(heading);
    for (const shop of entries) {
      const link = document.createElement("a");
      link.href = shop.url;
      link.target = "_blank";
      link.rel = "noreferrer";
      link.textContent = shop.name;
      els.shopsPanel.appendChild(link);
    }
  }
}

// User-initiated filter changes route through this wrapper; background shard
// loads call applyFilters directly, so only user actions snap the viewport
// to the top.
// Once the user has touched a control, a late-arriving option must not
// re-impose the stored value over their choice.
function userFilterChange() {
  state.view = nextViewState(state.view, { type: "filter", filters: currentFilters() });
  setCursor(state.view.cursor);
  writeFilters(window, state.view.filters);
  saveFilterURL();
  window.scrollTo(0, 0);
  applyFilters();
}

// Seller and category options are added as the index and shards arrive, so a
// stored value for either cannot be restored at start-up - the option does
// not exist yet. Retry as they appear, until the user takes over.
function restoreLateFilters() {
  state.view = nextViewState(state.view, {
    type: "restore-options",
    options: {
      seller: [...els.seller.options].map(option => option.value),
      category: [...els.category.options].map(option => option.value),
      sort: [...els.sort.options].map(option => option.value),
    },
  });
  // A pending search input still belongs to the control until its debounce.
  for (const key of /** @type {const} */ (["seller", "category", "sort"])) els[key].value = state.view.filters[key];
}

function clearFilters() {
  state.view = nextViewState(state.view, { type: "clear-filters" });
  const filters = state.view.filters;
  for (const key of /** @type {const} */ (["search", "seller", "category", "age", "sort", "priceMin", "priceMax", "collection", "tag", "kind"])) els[key].value = filters[key];
  for (const key of /** @type {const} */ (["newOnly", "favsOnly", "hideDups", "includeUnknown", "unreviewed", "changed"])) els[key].checked = filters[key];
  writeFilters(window, filters);
  saveFilterURL();
  setCursor(state.view.cursor);
  window.scrollTo(0, 0);
  applyFilters();
  els.search.focus();
}
els.filtersClear.addEventListener("click", clearFilters);

/** @type {number | ReturnType<typeof setTimeout>} */
let searchTimer = 0;
els.search.addEventListener("input", () => {
  clearTimeout(searchTimer);
  searchTimer = setTimeout(userFilterChange, 150);
});
for (const el of [els.seller, els.category, els.newOnly, els.favsOnly,
                  els.hideDups, els.age, els.sort, els.priceMin, els.priceMax, els.includeUnknown,
                  els.collection, els.tag, els.unreviewed, els.changed, els.kind]) {
  el.addEventListener("change", userFilterChange);
}

// Leaving group view is a "peek ended", not a filter change: put the user
// back where they were browsing instead of snapping to the top.
function exitGroupView() {
  state.view = nextViewState(state.view, { type: "close-group" });
  setCursor(state.view.cursor);
  // Order matters twice over. The group view leaves the page one row tall,
  // so scrolling back first would be clamped to the top; the full list has
  // to be restored before the browser will accept the old offset. And
  // renderGrid picks its window from the current scrollY, so it needs a
  // final pass after the scroll or the viewport lands on rows that were
  // never drawn.
  applyFilters();
  window.scrollTo(0, state.view.preGroupScroll);
  renderGrid();
  els.search.focus({ preventScroll: true });
}
els.groupClear.addEventListener("click", exitGroupView);

// The shops panel is a slide-over: it covers content, so it owns focus while
// open, says so on its toggle, and closes on Escape like any other overlay.
/** @param {boolean | string} open */
function setShopsOpen(open) {
  els.shopsPanel.hidden = !open;
  els.shopsToggle.setAttribute("aria-expanded", String(open));
  if (open) {
    const first = /** @type {HTMLElement | null} */ (els.shopsPanel.querySelector("button, a"));
    if (first) first.focus();
  } else {
    els.shopsToggle.focus();
  }
}
els.shopsToggle.addEventListener("click", () => setShopsOpen(els.shopsPanel.hidden));

window.addEventListener("keydown", (event) => {
  if (document.getElementById("album-preview").open) return;
  if (isSearchShortcut(event.key, document.activeElement && document.activeElement.tagName)) {
    event.preventDefault();
    els.search.focus();
    els.search.select();
    return;
  }
  if (event.key !== "Escape") return;
  // Innermost overlay first: a group view opened from inside the panel
  // should not close both at once.
  if (!els.shopsPanel.hidden) setShopsOpen(false);
  else if (state.view.groupKey) exitGroupView();
});

els.noteDismiss.addEventListener("click", () => {
  noteDismissed = true;
  els.note.hidden = true;
  els.search.focus();
});

// Theme: system, then an explicit light or dark that overrides it. Stored so
// the choice survives a reload, and reflected on <html> where the stylesheet
// reads it.
const THEME_CYCLE = ["system", "light", "dark"];
/** @type {Record<string, string>} */
const THEME_MARK = {
  system: "#mark-theme-system",
  light: "#mark-theme-light",
  dark: "#mark-theme-dark",
};
/** @type {Record<string, string>} */
const THEME_LABEL = {
  system: "Colour theme: follow system",
  light: "Colour theme: light",
  dark: "Colour theme: dark",
};

/** @param {string} theme */
function applyTheme(theme) {
  if (theme === "system") document.documentElement.removeAttribute("data-theme");
  else document.documentElement.setAttribute("data-theme", theme);
  els.themeIcon.setAttribute("href", THEME_MARK[theme]);
  els.themeToggle.setAttribute("aria-label", THEME_LABEL[theme]);
  els.themeToggle.title = THEME_LABEL[theme];
}

const storedTheme = readStored(window, "lamp.theme");
let theme = /** @type {string} */ (THEME_CYCLE.includes(/** @type {string} */ (storedTheme)) ? storedTheme : "system");
applyTheme(theme);
els.themeToggle.addEventListener("click", () => {
  theme = THEME_CYCLE[(THEME_CYCLE.indexOf(theme) + 1) % THEME_CYCLE.length];
  writeStored(window, "lamp.theme", theme);
  applyTheme(theme);
});

// The toolbar is sticky and wraps, so its height varies with viewport width.
// The grid uses it as scroll-margin: without it the skip link jumps the first
// row of cards to the top of the viewport, directly behind the toolbar it was
// meant to skip. Measured rather than hard-coded for that reason.
const topbar = document.getElementById("topbar");
function measureTopbar() {
  document.documentElement.style.setProperty("--topbar-h", `${topbar.offsetHeight}px`);
}
measureTopbar();
if (typeof ResizeObserver !== "undefined") {
  new ResizeObserver(measureTopbar).observe(topbar);
} else {
  window.addEventListener("resize", measureTopbar);
}

// Paint the loading state before the first fetch resolves, so the grid is
// never a blank rectangle with no explanation.
setStatus();
void loadAll();
