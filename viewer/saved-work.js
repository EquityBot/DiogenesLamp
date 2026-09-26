"use strict";

// Local browser work is separate from fetched catalogue metadata.
const workEl = id => document.getElementById(id);
const preview = workEl("album-preview");
let previewIds = [];
let previewAt = 0;
let previewReturnId = "";
let previewInitial = {};

function actionStatus(text) {
  workEl("action-status").textContent = text;
  workEl("action-status").hidden = !text;
}

function saveFilterURL() {
  const query = filtersToURL(state.view.filters);
  if (location.search !== query) history.pushState(null, "", query);
}

function selectOptions(select, values, wanted = select.value) {
  const first = select.options[0].cloneNode(true);
  select.replaceChildren(first);
  for (const value of [...new Set(values)].filter(Boolean).sort()) {
    select.add(new Option(value, value));
  }
  // Keep a missing filter explicit: an imported collection may refer to an
  // unavailable shard. Silently clearing it would display unrelated albums.
  if (wanted && ![...select.options].some(o => o.value === wanted)) {
    select.add(new Option(wanted + " (unavailable)", wanted));
  }
  select.value = wanted;
}

function refreshSavedState(render = true) {
  const entries = userStore.load();
  const section = kind => new Map(entries.filter(e => e.kind === kind).map(e => [e.id, e.value]));
  state.favorites = new Set(entries.filter(e => e.kind === "favorite" && e.value).map(e => e.id));
  state.collections = section("collection");
  state.tags = section("tags");
  state.reviews = section("review");
  selectOptions(els.collection, [...state.collections.values()], state.view.filters.collection);
  selectOptions(els.tag, [...state.tags.values()].flat(), state.view.filters.tag);
  selectOptions(workEl("saved-search"), entries.filter(e => e.kind === "search" && e.value).map(e => e.id));
  if (render) applyFilters();
  if (preview.open) updatePreviewFavourite();
}

function useFilters(filters) {
  const saved = normalizeFilters(filters);
  refreshSavedState(false);
  for (const [key, value] of Object.entries(saved)) {
    if (!els[key]) continue;
    if (typeof value === "boolean") els[key].checked = value;
    else {
      if (els[key].tagName === "SELECT" && value &&
          ![...els[key].options].some(o => o.value === value)) {
        els[key].add(new Option(value + " (unavailable)", value));
      }
      els[key].value = value;
    }
  }
  state.view = nextViewState(state.view, {type: "filter", filters: saved});
  writeFilters(window, saved);
  window.scrollTo(0, 0);
  applyFilters();
}
window.addEventListener("popstate", () => useFilters(filtersFromURL(location.search) || defaultFilters()));
window.addEventListener("storage", event => {
  if (event.key === null || event.key.startsWith(USER_PREFIX)) refreshSavedState();
});

function downloadWork(text, name, type) {
  const url = URL.createObjectURL(new Blob([text], {type}));
  const link = document.createElement("a");
  link.href = url;
  link.download = name;
  document.body.appendChild(link);
  link.click();
  link.remove();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

function catalogueAvailability() {
  const loading = !state.index || state.loadedSellers < state.index.sellers.length;
  return loading ? "partial: still loading" : state.failedShards.length
    ? "partial: unavailable sellers " + state.failedShards.join(", ") : "complete";
}

els.exportFavs.addEventListener("click", () => {
  const entries = userStore.load();
  const ids = new Set(entries.filter(e => ["favorite", "note", "tags", "collection"].includes(e.kind))
    .map(e => e.id));
  const albums = state.albums.filter(a => ids.has(a.id)).map(a => ({
    id: a.id, seller: a.seller, title: a.title, price: a.price, album_url: a.album_url,
    links: a.links, category: a.category, updated_at: a.updated_at,
  }));
  const present = new Set(albums.map(a => a.id));
  const missingIds = [...ids].filter(id => !present.has(id));
  const backup = {format: "diogeneslamp", version: 1, exported_at: new Date().toISOString(),
    catalogue_status: catalogueAvailability(), entries, albums, unresolved_ids: missingIds};
  downloadWork(JSON.stringify(backup, null, 2), "diogenes-saved-work.json", "application/json");
  actionStatus(`Backed up ${entries.length} saved fields. ${missingIds.length} album IDs have no loaded metadata; their saved work is included.`);
});

workEl("import-backup").addEventListener("change", async event => {
  const file = event.target.files[0];
  if (!file) return;
  try {
    if (file.size > 20 * 1024 * 1024) throw new Error("Backup exceeds 20 MB.");
    const count = importUserBackup(userStore, JSON.parse(await file.text()));
    actionStatus(`Imported ${count} saved fields. Existing values were kept.`);
  } catch (error) { savedDataError(error.message); }
  finally { refreshSavedState(); event.target.value = ""; }
});

workEl("export-csv").addEventListener("click", () => {
  const availability = catalogueAvailability();
  const scope = `${availability}; current results; ${filtersToURL(state.view.filters)}`;
  downloadWork(catalogueCsv(state.filtered, scope), "diogenes-results.csv", "text/csv;charset=utf-8");
  actionStatus(`Exported ${state.filtered.length} current results. Catalogue: ${availability}.`);
});
workEl("save-search").addEventListener("click", () => {
  const name = workEl("search-name").value.trim();
  if (!name) { workEl("search-name").focus(); actionStatus("Enter a name for this search."); return; }
  if (userStore.set("search", name, currentFilters())) {
    refreshSavedState(false);
    workEl("saved-search").value = name;
    actionStatus("Search saved.");
  }
});
workEl("saved-search").addEventListener("change", event => {
  const name = event.target.value;
  const filters = userStore.get("search", name);
  if (filters) { useFilters(filters); saveFilterURL(); workEl("search-name").value = name; }
});
workEl("remove-search").addEventListener("click", () => {
  const name = workEl("saved-search").value;
  if (name && userStore.set("search", name, null)) {
    workEl("saved-search").value = "";
    refreshSavedState(false);
    actionStatus("Saved search removed.");
  }
});
workEl("mark-reviewed").addEventListener("click", () => {
  const seller = els.seller.value;
  if (!seller || !state.shardAlbums.has(seller) || state.failedShards.includes(seller)) {
    actionStatus("Choose a seller whose catalogue has finished loading.");
    return;
  }
  // Use the snapshot timestamp, not the wall clock: a later downloaded
  // catalogue can contain changes already older than today's browsing time.
  const cutoff = state.shardAlbums.get(seller).reduce((latest, album) => {
    const date = Date.parse(album.last_seen || album.updated_at || "");
    return Number.isFinite(date) ? Math.max(latest, date) : latest;
  }, 0) || Date.parse(state.generatedAt);
  if (!Number.isFinite(cutoff)) { actionStatus("This catalogue has no valid review date."); return; }
  if (userStore.set("review", seller, cutoff)) {
    refreshSavedState();
    actionStatus(`${seller} marked reviewed through ${new Date(cutoff).toISOString()}.`);
  }
});

function previewAlbum() { return state.albums.find(a => a.id === previewIds[previewAt]); }
function updatePreviewFavourite() {
  const fav = workEl("preview-favourite");
  const chosen = state.favorites.has(previewIds[previewAt]);
  fav.setAttribute("aria-pressed", String(chosen));
  fav.textContent = favLabel(chosen);
}
function localDate(value) { return value ? String(value).replace("T", " ") : "Not recorded"; }

function showPreview() {
  const album = previewAlbum();
  if (!album) return;
  workEl("preview-title").textContent = album.title;
  workEl("preview-position").textContent = `${previewAt + 1} of ${previewIds.length}`;
  workEl("preview-prev").disabled = previewAt === 0;
  workEl("preview-next").disabled = previewAt + 1 === previewIds.length;
  workEl("preview-facts").textContent = `${album.seller} · Album ${album.id} · ${album.category || "No category"}` +
    `\nFirst seen: ${localDate(album.first_seen)}\nListed: ${localDate(album.last_seen)}\nDetails checked: ${localDate(album.updated_at)}` +
    (album.gone_since ? `\nNo longer listed since ${localDate(album.gone_since)}` : "");
  const reason = compileCatalogueSearch(state.view.filters.search).reason(album);
  workEl("preview-match").textContent = reason ? "Search match: " + reason : "";
  const evidence = album.price_evidence;
  workEl("preview-price").textContent = album.price ? `CNY ${album.price}` +
    (evidence?.token && evidence?.rule ? ` · ${evidence.field || "album text"}: ${evidence.token} (${evidence.rule})` +
      (evidence.candidates?.length > 1 ? ` · Other candidates: ${evidence.candidates.join(", ")}` : "") : " · Extraction evidence not recorded")
    : "Price unknown";
  const run = state.index?.sellers.find(s => s.id === album.seller)?.run;
  workEl("preview-run").textContent = run ? `Seller last successful run: ${localDate(run.last_success)}` +
    (run.failed_ids?.length ? ` · ${run.failed_ids.length} detail failures awaiting retry` : "") : "Seller run status not recorded";
  const cover = workEl("preview-cover");
  cover.hidden = !album.thumb;
  workEl("preview-no-cover").hidden = Boolean(album.thumb);
  cover.onerror = () => { cover.hidden = true; workEl("preview-no-cover").hidden = false; };
  if (album.thumb) cover.src = `data/${album.thumb}?v=${thumbVersion(album)}`;
  else cover.removeAttribute("src");
  workEl("preview-links").replaceChildren();
  for (const link of [{platform: "Original album", url: album.album_url}, ...album.links]) {
    const url = new URL(link.url, location.href);
    if (!["https:", "http:"].includes(url.protocol)) continue;
    const a = document.createElement("a");
    a.href = url.href; a.target = "_blank"; a.rel = "noreferrer"; a.textContent = link.platform;
    workEl("preview-links").appendChild(a);
  }
  const changes = [...(album.history || [])].reverse();
  workEl("preview-history").replaceChildren();
  for (const change of changes) {
    const li = document.createElement("li");
    li.textContent = `${localDate(change.at)}: ${change.field}: ${JSON.stringify(change.before)} → ${JSON.stringify(change.after)}`;
    workEl("preview-history").appendChild(li);
  }
  if (!changes.length) workEl("preview-history").textContent = "No changes recorded.";
  previewInitial = {
    collection: userStore.get("collection", album.id) || "",
    tags: (userStore.get("tags", album.id) || []).join(", "),
    notes: userStore.get("note", album.id) || "",
  };
  for (const [field, value] of Object.entries(previewInitial)) workEl("preview-" + field).value = value;
  workEl("preview-save-status").textContent = "";
  workEl("preview-similar").open = false;
  workEl("similar-results").replaceChildren();
  updatePreviewFavourite();
}

function openPreview(id) {
  previewIds = state.filtered.map(a => a.id);
  previewAt = previewIds.indexOf(id);
  previewReturnId = id;
  showPreview();
  document.body.classList.add("preview-open");
  preview.showModal();
}
function previewDirty() {
  return Object.entries(previewInitial).some(([key, value]) => workEl("preview-" + key).value !== value);
}
function leavePreviewAllowed() {
  return !previewDirty() || window.confirm("Discard the unsaved notes, tags or collection for this album?");
}
function stepPreview(delta) {
  const next = previewAt + delta;
  if (next < 0 || next >= previewIds.length || !leavePreviewAllowed()) return;
  previewAt = next;
  showPreview();
}
workEl("preview-prev").addEventListener("click", () => stepPreview(-1));
workEl("preview-next").addEventListener("click", () => stepPreview(1));
workEl("preview-close").addEventListener("click", () => { if (leavePreviewAllowed()) preview.close(); });
preview.addEventListener("cancel", event => { if (!leavePreviewAllowed()) event.preventDefault(); });
preview.addEventListener("close", () => {
  document.body.classList.remove("preview-open");
  const at = state.filtered.findIndex(a => a.id === previewReturnId);
  if (at >= 0) moveCursor(at);
  else els.grid.focus({preventScroll: true});
});
preview.addEventListener("keydown", event => {
  if (event.ctrlKey || event.metaKey || event.altKey) return;
  if (event.target.closest("input, textarea, select")) return;
  if (["ArrowLeft", "ArrowRight"].includes(event.key)) {
    event.preventDefault(); stepPreview(event.key === "ArrowLeft" ? -1 : 1);
  }
});
workEl("preview-favourite").addEventListener("click", event => toggleFavorite(previewIds[previewAt], event.currentTarget));
workEl("preview-form").addEventListener("submit", event => {
  event.preventDefault();
  const id = previewIds[previewAt];
  const tags = workEl("preview-tags").value.split(",").map(s => s.trim()).filter(Boolean);
  if (tags.length > 20 || tags.some(tag => tag.length > 50)) {
    workEl("preview-save-status").textContent = "Use at most 20 tags, each no longer than 50 characters.";
    return;
  }
  // Only write edited fields. An unchanged open form must not revert a
  // different field saved by another tab after this preview opened.
  for (const [field, kind] of [["collection", "collection"], ["tags", "tags"], ["notes", "note"]]) {
    const value = workEl("preview-" + field).value;
    if (value === previewInitial[field]) continue;
    if (!userStore.set(kind, id, field === "tags" ? [...new Set(tags)] : value)) {
      workEl("preview-save-status").textContent = "Save incomplete. Keep this form open and export a backup.";
      return;
    }
    previewInitial[field] = value;
  }
  refreshSavedState();
  workEl("preview-save-status").textContent = "Saved.";
});

refreshSavedState(false);
if (!filtersFromURL(location.search)) {
  history.replaceState(null, "", filtersToURL(state.view.storedFilters));
}

workEl("preview-similar").addEventListener("toggle", () => {
  if (!workEl("preview-similar").open) return;
  const album = previewAlbum();
  const dismissed = new Set(userStore.load().filter(e => e.kind === "dismiss" && e.value === true &&
    e.id.startsWith(album.id + ":")).map(e => e.id.slice(album.id.length + 1)));
  const matches = similarCovers(state.albums, album, dismissed);
  const list = workEl("similar-results");
  list.replaceChildren();
  if (!matches.length) list.textContent = album.cover_hash ? "No suggestions within the similarity threshold." :
    "No cover fingerprint. Run lamp prepare-covers for your cached catalogue to try this experiment.";
  for (const match of matches) {
    const li = document.createElement("li");
    const view = document.createElement("button");
    view.type = "button";
    view.textContent = `${match.album.title} (${match.distance}/64 differences)`;
    view.addEventListener("click", () => {
      if (!leavePreviewAllowed()) return;
      if (!previewIds.includes(match.album.id)) previewIds.push(match.album.id);
      previewAt = previewIds.indexOf(match.album.id);
      showPreview();
      workEl("preview-close").focus();
    });
    const dismiss = document.createElement("button");
    dismiss.type = "button";
    dismiss.textContent = "Dismiss";
    dismiss.setAttribute("aria-label", "Dismiss similar cover: " + match.album.title);
    dismiss.addEventListener("click", () => {
      if (userStore.set("dismiss", album.id + ":" + match.album.id, true)) {
        li.remove(); workEl("preview-similar").querySelector("summary").focus();
      }
    });
    li.append(view, dismiss); list.appendChild(li);
  }
});
window.addEventListener("beforeunload", event => {
  if (preview.open && previewDirty()) { event.preventDefault(); event.returnValue = ""; }
});
