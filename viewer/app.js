"use strict";

// Reads the legacy candle.favorites key once so pre-rename stars survive.
// A corrupt value must not throw here: this runs while the module body is
// still executing, so an exception would leave no listeners bound and no
// catalog loaded - a blank page recoverable only by clearing site data.
function loadFavorites() {
  for (const key of ["lamp.favorites", "candle.favorites"]) {
    const raw = localStorage.getItem(key);
    if (!raw) continue;
    try {
      const parsed = JSON.parse(raw);
      if (Array.isArray(parsed)) return new Set(parsed);
    } catch {
      console.warn(`ignoring unreadable ${key}`);
    }
  }
  return new Set();
}

const state = {
  index: null,
  albums: [],          // all loaded albums, in shard order
  filtered: [],
  favorites: loadFavorites(),
  loadedSellers: 0,
  pendingShards: [],   // sellers whose shards are not yet fetched
  generatedAt: "",
  dupCounts: new Map(), // dup_key -> group size, valid for current filter pass
  dupHidden: 0,
  dupGroup: null,      // dup_key being inspected via a x-N badge click
  preGroupScroll: 0,   // scrollY to restore when leaving group view
  bannedKeys: new Set(), // corpus-global category-label keys, grown per shard
  titleKeyCounts: new Map(), // running t:-key counts feeding bannedKeys
  shardAlbums: new Map(), // seller id -> annotated albums, in fetch order
  failedShards: [],    // sellers whose shard could not be fetched
  categories: new Set(), // every category seen, collected as shards arrive
  filterVersion: 0,    // bumped whenever `filtered` is rebuilt
};

const els = {
  search: document.getElementById("search"),
  seller: document.getElementById("seller-filter"),
  category: document.getElementById("category-filter"),
  newOnly: document.getElementById("new-only"),
  favsOnly: document.getElementById("favs-only"),
  hideDups: document.getElementById("hide-dups"),
  age: document.getElementById("age-filter"),
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
function currentFilters() {
  return {
    search: els.search.value,
    seller: els.seller.value,
    category: els.category.value,
    age: els.age.value,
    newOnly: els.newOnly.checked,
    favsOnly: els.favsOnly.checked,
    hideDups: els.hideDups.checked,
  };
}

// Restoring a seller or category that the index no longer offers would leave
// the control blank while the filter still applied, so only values the
// dropdown actually holds are written back. The options for both arrive
// later than this runs, which is why applyStoredFilters is called again as
// they are added.
function setSelectIfPresent(select, value) {
  if (!value) return;
  if ([...select.options].some((option) => option.value === value)) {
    select.value = value;
  }
}

const storedFilters = readFilters(localStorage);

function applyStoredFilters() {
  els.search.value = storedFilters.search;
  els.age.value = storedFilters.age;
  els.newOnly.checked = storedFilters.newOnly;
  els.favsOnly.checked = storedFilters.favsOnly;
  els.hideDups.checked = storedFilters.hideDups;
  setSelectIfPresent(els.seller, storedFilters.seller);
  setSelectIfPresent(els.category, storedFilters.category);
}
applyStoredFilters();

function setStatus() {
  const total = state.index ? state.index.sellers.length : 0;
  const loading = !state.index || state.loadedSellers < total;
  const dups = state.dupHidden
    ? ` | ${state.dupHidden.toLocaleString()} duplicates hidden` : "";
  // A seller whose scrape errored has no shard on disk, so its fetch 404s.
  // Those still count in loadedSellers (the attempt finished), so name the
  // missing part rather than silently reporting a full catalog.
  const missing = state.failedShards.length
    ? ` | ${state.failedShards.length} unavailable` : "";
  els.status.textContent =
    `${state.filtered.length.toLocaleString()} albums | ` +
    `${state.loadedSellers}/${total} sellers loaded${missing}${dups}`;

  els.progress.style.width = total ? `${(state.loadedSellers / total) * 100}%` : "0";
  els.progress.parentElement.hidden = !loading;
  // Announcing an incomplete grid as ready would tell a screen reader to
  // read a list that is still growing under it.
  els.grid.setAttribute("aria-busy", String(loading));
  els.filtersClear.hidden = !anyFilterActive(currentFilters()) || Boolean(state.dupGroup);
  setGridMessage(loading);
  setLoadNote(loading);
}

// The blank grid is the single most confusing state in the viewer: it looks
// identical whether nothing has loaded yet, nothing matched, or every match
// was collapsed as a duplicate. Name which one it is.
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
    text = "No albums match these filters.";
  }
  els.message.textContent = text;
  els.message.hidden = false;
}

// Failed shards used to live in a title attribute, which a keyboard or touch
// user never sees. Surface them once the load settles, and let them be
// dismissed rather than sitting there for the session.
let noteDismissed = false;
function setLoadNote(loading) {
  const failed = state.failedShards.length;
  if (loading || !failed || noteDismissed) {
    els.note.hidden = true;
    return;
  }
  const names = state.failedShards.join(", ");
  els.noteText.textContent =
    `${failed} ${failed === 1 ? "seller has" : "sellers have"} no data on disk ` +
    `and ${failed === 1 ? "was" : "were"} left out: ${names}.`;
  els.note.hidden = false;
}

const SHARD_CONCURRENCY = 6;

async function loadAll() {
  let index;
  try {
    const response = await fetch("data/index.json");
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    index = await response.json();
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
      const shard = await response.json();
      for (const album of shard.albums) {
        album.dup_key = dupKey(album);
        album.title_lc = (album.title || "").toLowerCase();
        album.published_ms = publishedMs(album);
      }
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
  state.albums = [];
  for (const seller of state.index.sellers) {
    const albums = state.shardAlbums.get(seller.id);
    if (albums) state.albums.push(...albums);
  }
}

// With several shard fetches landing concurrently, refreshing the grid per
// shard would filter and re-render the whole corpus dozens of times a
// second. Coalesce to at most one refresh per 200ms.
let refreshTimer = 0;
function scheduleRefresh() {
  if (refreshTimer) return;
  refreshTimer = setTimeout(refreshUI, 200);
}

function refreshUI() {
  clearTimeout(refreshTimer);
  refreshTimer = 0;
  rebuildAlbums();
  refreshCategoryOptions();
  applyFilters();
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

function applyFilters() {
  state.dupCounts = new Map();
  state.dupHidden = 0;
  if (state.dupGroup) {
    // Group view: every copy behind one x-N badge, all other filters ignored.
    state.filtered = state.albums.filter((a) => a.dup_key === state.dupGroup);
    state.filterVersion += 1;
    els.groupClear.textContent =
      `✕ showing ${state.filtered.length} copies`;
    els.groupClear.hidden = false;
    setStatus();
    renderGrid();
    return;
  }
  els.groupClear.hidden = true;
  const query = els.search.value.trim().toLowerCase();
  const seller = els.seller.value;
  const category = els.category.value;
  const newOnly = els.newOnly.checked;
  const favsOnly = els.favsOnly.checked;
  // Age filter: postings older than the chosen window are likely dead
  // stock. Cutoff uses the real scraped date when a record has one, the
  // album-id estimate otherwise (see age.js), and counts whole days so it
  // agrees with what `lamp scrape --max-age-days` chose to fetch.
  const maxDays = parseInt(els.age.value, 10) || 0;
  const now = Date.now();
  let filtered = state.albums.filter((album) =>
    (!seller || album.seller === seller) &&
    (!category || album.category === category) &&
    (!newOnly || album.first_seen === state.generatedAt) &&
    (!favsOnly || state.favorites.has(album.id)) &&
    (!maxDays || ageInDays(album.published_ms, now) <= maxDays) &&
    (!query || album.title_lc.includes(query))
  );
  if (els.hideDups.checked) {
    const before = filtered.length;
    const { out, counts } = collapseDups(filtered, state.bannedKeys);
    filtered = out;
    state.dupCounts = counts;
    state.dupHidden = before - out.length;
  }
  state.filtered = filtered;
  state.filterVersion += 1;
  setStatus();
  // Not a forced rebuild: the render reuses cards by album, and any whose
  // duplicate badge no longer matches is rebuilt individually. During a
  // streaming load this runs several times a second, and discarding every
  // card each time made the browser re-decode covers it already had.
  renderGrid();
}

// Text content only: the DOM escapes &, < and > but leaves quotes alone,
// which is fine between tags and unsafe inside an attribute.
function escapeHtml(text) {
  const div = document.createElement("div");
  div.textContent = text;
  return div.innerHTML;
}

// Attribute values need the quotes escaped too, or a value containing one
// closes the attribute early and everything after it is parsed as markup.
function escapeAttr(value) {
  return escapeHtml(value ?? "").replace(/"/g, "&quot;").replace(/'/g, "&#39;");
}

// Stable hue per album, so a cover-less card keeps the same colour across
// reloads and neighbouring cards rarely match. Any cheap string hash does;
// this is FNV-1a folded onto the colour wheel.
function coverHue(seed) {
  let hash = 2166136261;
  for (let i = 0; i < seed.length; i += 1) {
    hash ^= seed.charCodeAt(i);
    hash = Math.imul(hash, 16777619);
  }
  return Math.abs(hash) % 360;
}

function thumbVersion(album) {
  return encodeURIComponent(album.date_modified || album.updated_at || "");
}

function makeCard(album) {
  const card = document.createElement("div");
  card.className = "card";
  const isNew = album.first_seen === state.generatedAt;
  const isFav = state.favorites.has(album.id);
  // Everything interpolated below reaches us from scraped remote HTML. The
  // scraper constrains platform to a known set and builds URLs itself, so
  // nothing hostile is expected - but escaping is a line of code and the
  // alternative is trusting that forever.
  const links = album.links.map((link) =>
    `<a class="${escapeAttr(link.platform)}" href="${escapeAttr(link.url)}" ` +
    `target="_blank" rel="noreferrer">${escapeHtml(link.platform)}</a>`
  ).join("");
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
    `<div class="card-cover" style="--cover-hue:${coverHue(album.id)}">` +
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
    // The badges live inside the cover so they are positioned against it
    // rather than against a hard-coded offset that has to be kept in step
    // with the cover height at every breakpoint.
    (isNew ? `<span class="badge-new">NEW</span>` : "") +
    (dupCount > 1
      ? `<button class="badge-dup" type="button" ` +
        `aria-label="Show all ${dupCount} copies of this posting">` +
        `&times;${dupCount}</button>`
      : "") +
    `<button class="fav-btn" type="button" aria-pressed="${isFav}" ` +
    `aria-label="${favLabel(isFav)}">` +
    `<span aria-hidden="true">${favGlyph(isFav)}</span></button>` +
    `</div>` +
    `<div class="card-title">${escapeHtml(album.title)}</div>` +
    `<div class="card-meta"><span class="seller-tag">${escapeHtml(album.seller)}</span>` +
    `<span class="card-links">${links}</span>` +
    `<a class="album-link" href="${escapeAttr(album.album_url)}" target="_blank" ` +
    `rel="noreferrer">Album</a></div>`;
  // An inline onerror attribute did this before. Binding it here keeps the
  // markup free of executable attributes, which is what lets a strict
  // script-src policy apply to this page unchanged.
  const thumb = card.querySelector(".card-thumb");
  if (thumb) {
    thumb.addEventListener("error", (event) => {
      event.currentTarget.style.display = "none";
    });
  }
  card.querySelector(".fav-btn").addEventListener("click", (event) => {
    toggleFavorite(album.id, event.currentTarget);
  });
  const dupBtn = card.querySelector(".badge-dup");
  if (dupBtn) {
    dupBtn.addEventListener("click", () => {
      state.preGroupScroll = window.scrollY;
      state.dupGroup = album.dup_key;
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
function favLabel(isFav) {
  return isFav ? "Remove from favourites" : "Add to favourites";
}

// Filled versus hollow star. aria-pressed covers screen readers, but the two
// states used to differ by colour alone, which is invisible to a colour-blind
// user and measured 1.19:1 between them - well under the 3:1 a state
// indicator needs.
function favGlyph(isFav) {
  return isFav ? "★" : "☆";
}

function toggleFavorite(albumId, button) {
  if (state.favorites.has(albumId)) {
    state.favorites.delete(albumId);
  } else {
    state.favorites.add(albumId);
  }
  const isFav = state.favorites.has(albumId);
  button.setAttribute("aria-pressed", String(isFav));
  button.setAttribute("aria-label", favLabel(isFav));
  // The click path mutates this node in place; the scroll path rebuilds it
  // from state.favorites. Both have to paint the same glyph or a card
  // recycled after a scroll disagrees with the one the user just clicked.
  button.firstElementChild.textContent = favGlyph(isFav);
  localStorage.setItem("lamp.favorites", JSON.stringify([...state.favorites]));
  if (els.favsOnly.checked) applyFilters();
}

// Card geometry lives in style.css and is read from there. Duplicating the
// numbers here would let a CSS tweak silently desynchronise the absolute
// positions this file computes, leaving overlapping or gapped rows.
// Re-read rather than cached once: the card size changes at the narrow
// breakpoint, and a stale copy would place every card on the old pitch,
// leaving overlapping rows on a phone or after a window resize.
function readGeometry() {
  const css = getComputedStyle(document.documentElement);
  const px = (name, fallback) => parseInt(css.getPropertyValue(name), 10) || fallback;
  const gap = px("--gap", 12);
  return { w: px("--card-w", 220) + gap, h: px("--card-h", 300) + gap, gap };
}
let geometry = readGeometry();

function columns() {
  const width = els.container.clientWidth - geometry.gap * 2;
  return Math.max(1, Math.floor(width / geometry.w));
}
function rowHeight() { return geometry.h; }
function rowsFor(count) { return Math.ceil(count / columns()); }
function placeCard(i) {
  const cols = columns();
  const x = (i % cols) * geometry.w;
  const y = Math.floor(i / cols) * geometry.h;
  return `translate(${x}px, ${y}px)`;
}

// Windowed rendering: only rows near the viewport exist in the DOM.
const OVERSCAN_ROWS = 3;
let lastRange = { start: -1, end: -1, cols: -1, count: -1, version: -1 };
// Cards currently in the grid. Scroll-driven renders keep them (same nodes,
// listeners, decoded thumbs) and touch only the rows entering and leaving
// the window; anything that can change card content or membership goes
// through renderGrid(force = true), which rebuilds from scratch.
// Keyed by seller and id together: an id repeated in one view would
// otherwise claim the same node twice and strand a card in the DOM.
let cardCache = new Map();
const cardKey = (album) => `${album.seller} ${album.id}`;

function renderGrid(force = false) {
  const cols = columns();
  const rowH = rowHeight();
  const totalRows = rowsFor(state.filtered.length);
  els.spacer.style.height = `${totalRows * rowH}px`;

  // Measured from the spacer, not its container: the load note and the empty
  // message sit above it inside the same container, so anchoring on the
  // container would offset every card by whatever those occupy.
  const viewTop = window.scrollY - els.spacer.offsetTop;
  const firstRow = Math.max(0, Math.floor(viewTop / rowH) - OVERSCAN_ROWS);
  const lastRow = Math.min(
    totalRows,
    Math.ceil((viewTop + window.innerHeight) / rowH) + OVERSCAN_ROWS
  );
  // filterVersion is part of the key: two different filters can produce the
  // same row range and the same count, and without it the second would be
  // mistaken for a no-op and never drawn.
  if (!force && firstRow === lastRange.start && lastRow === lastRange.end &&
      cols === lastRange.cols && state.filtered.length === lastRange.count &&
      state.filterVersion === lastRange.version) {
    return;
  }
  lastRange = { start: firstRow, end: lastRow, cols, count: state.filtered.length,
                version: state.filterVersion };

  if (force) {
    els.grid.replaceChildren();
    cardCache = new Map();
  }
  // Apply only the difference. Re-appending every card each scroll step
  // re-parented ~150 nodes to move by one row; now the cards that stay put
  // are left alone and only the entering and leaving rows are touched.
  const next = new Map();
  const startIdx = firstRow * cols;
  const endIdx = Math.min(state.filtered.length, lastRow * cols);
  for (let i = startIdx; i < endIdx; i += 1) {
    const album = state.filtered[i];
    const key = cardKey(album);
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
    }
    if (!card) {
      card = makeCard(album);
      els.grid.appendChild(card);
    }
    const transform = placeCard(i);
    if (card.style.transform !== transform) card.style.transform = transform;
    // Only a window of rows exists in the DOM, so role=list would otherwise
    // advertise a set the size of that window: "item 1 of 30" against a
    // catalog of six figures. Stamped here rather than in makeCard because a
    // cached card is reused at a different index when the filter changes,
    // and only this loop knows the new one.
    setIfChanged(card, "aria-setsize", String(state.filtered.length));
    setIfChanged(card, "aria-posinset", String(i + 1));
    next.set(key, card);
  }

  // Remove departed cards before reordering, so the order pass never sees a
  // node that is on its way out. A card holding focus is not simply dropped:
  // that leaves the keyboard user on <body> with their place lost, which
  // happens both when they scroll a focused card out of the window and when
  // un-favouriting a card removes it from a favourites-only view.
  const departed = [...cardCache.values()];
  rescueFocus(departed, next);
  for (const node of departed) node.remove();

  // DOM order has to match visual order or Tab walks the grid in the order
  // rows happened to be appended. Scrolling up appends the entering rows
  // after the surviving ones, so tabbing threw the viewport backwards by
  // whole screens. Surviving cards are already in relative order, so this
  // moves only the cards that actually entered.
  let previous = null;
  for (const card of next.values()) {
    if (card.previousElementSibling !== previous) {
      els.grid.insertBefore(card, previous ? previous.nextSibling : els.grid.firstChild);
    }
    previous = card;
  }
  cardCache = next;
}

function setIfChanged(el, name, value) {
  if (el.getAttribute(name) !== value) el.setAttribute(name, value);
}

// Focus follows the user, so hand it to the same control on the card that
// took the removed one's place; failing that, park it on the grid itself,
// which is focusable for the skip link anyway. preventScroll throughout:
// this runs mid-render and must not fight the scroll that triggered it.
function rescueFocus(departed, survivors) {
  const active = document.activeElement;
  if (!active || !departed.some((node) => node.contains(active))) return;
  const selector = active.classList.contains("fav-btn") ? ".fav-btn"
    : active.classList.contains("badge-dup") ? ".badge-dup"
      : active.classList.contains("album-link") ? ".album-link" : null;
  if (selector) {
    for (const card of survivors.values()) {
      const target = card.querySelector(selector);
      if (target) {
        target.focus({ preventScroll: true });
        return;
      }
    }
  }
  els.grid.focus({ preventScroll: true });
}

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

function renderShops(shops) {
  const groups = new Map();
  for (const shop of shops) {
    if (!groups.has(shop.platform)) groups.set(shop.platform, []);
    groups.get(shop.platform).push(shop);
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
let userTouchedFilters = false;

function userFilterChange() {
  userTouchedFilters = true;
  state.dupGroup = null;
  writeFilters(localStorage, currentFilters());
  window.scrollTo(0, 0);
  applyFilters();
}

// Seller and category options are added as the index and shards arrive, so a
// stored value for either cannot be restored at start-up - the option does
// not exist yet. Retry as they appear, until the user takes over.
function restoreLateFilters() {
  if (userTouchedFilters) return;
  const before = `${els.seller.value}|${els.category.value}`;
  setSelectIfPresent(els.seller, storedFilters.seller);
  setSelectIfPresent(els.category, storedFilters.category);
  return before !== `${els.seller.value}|${els.category.value}`;
}

function clearFilters() {
  userTouchedFilters = true;
  const defaults = defaultFilters();
  els.search.value = defaults.search;
  els.seller.value = defaults.seller;
  els.category.value = defaults.category;
  els.age.value = defaults.age;
  els.newOnly.checked = defaults.newOnly;
  els.favsOnly.checked = defaults.favsOnly;
  els.hideDups.checked = defaults.hideDups;
  writeFilters(localStorage, defaults);
  state.dupGroup = null;
  window.scrollTo(0, 0);
  applyFilters();
  els.search.focus();
}
els.filtersClear.addEventListener("click", clearFilters);

let searchTimer = 0;
els.search.addEventListener("input", () => {
  clearTimeout(searchTimer);
  searchTimer = setTimeout(userFilterChange, 150);
});
for (const el of [els.seller, els.category, els.newOnly, els.favsOnly,
                  els.hideDups, els.age]) {
  el.addEventListener("change", userFilterChange);
}

// Leaving group view is a "peek ended", not a filter change: put the user
// back where they were browsing instead of snapping to the top.
function exitGroupView() {
  state.dupGroup = null;
  // Order matters twice over. The group view leaves the page one row tall,
  // so scrolling back first would be clamped to the top; the full list has
  // to be restored before the browser will accept the old offset. And
  // renderGrid picks its window from the current scrollY, so it needs a
  // final pass after the scroll or the viewport lands on rows that were
  // never drawn.
  applyFilters();
  window.scrollTo(0, state.preGroupScroll);
  renderGrid();
  els.search.focus({ preventScroll: true });
}
els.groupClear.addEventListener("click", exitGroupView);

// The shops panel is a slide-over: it covers content, so it owns focus while
// open, says so on its toggle, and closes on Escape like any other overlay.
function setShopsOpen(open) {
  els.shopsPanel.hidden = !open;
  els.shopsToggle.setAttribute("aria-expanded", String(open));
  if (open) {
    const first = els.shopsPanel.querySelector("button, a");
    if (first) first.focus();
  } else {
    els.shopsToggle.focus();
  }
}
els.shopsToggle.addEventListener("click", () => setShopsOpen(els.shopsPanel.hidden));

window.addEventListener("keydown", (event) => {
  if (event.key !== "Escape") return;
  // Innermost overlay first: a group view opened from inside the panel
  // should not close both at once.
  if (!els.shopsPanel.hidden) setShopsOpen(false);
  else if (state.dupGroup) exitGroupView();
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
const THEME_ICON = { system: "◑", light: "☀", dark: "☽" };
const THEME_LABEL = {
  system: "Colour theme: follow system",
  light: "Colour theme: light",
  dark: "Colour theme: dark",
};

function applyTheme(theme) {
  if (theme === "system") document.documentElement.removeAttribute("data-theme");
  else document.documentElement.setAttribute("data-theme", theme);
  els.themeIcon.textContent = THEME_ICON[theme];
  els.themeToggle.setAttribute("aria-label", THEME_LABEL[theme]);
  els.themeToggle.title = THEME_LABEL[theme];
}

let theme = THEME_CYCLE.includes(localStorage.getItem("lamp.theme"))
  ? localStorage.getItem("lamp.theme") : "system";
applyTheme(theme);
els.themeToggle.addEventListener("click", () => {
  theme = THEME_CYCLE[(THEME_CYCLE.indexOf(theme) + 1) % THEME_CYCLE.length];
  try {
    localStorage.setItem("lamp.theme", theme);
  } catch {
    // Persistence is a convenience; the session still honours the choice.
  }
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
loadAll();
