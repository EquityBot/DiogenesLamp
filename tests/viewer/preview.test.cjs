"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");
const vm = require("node:vm");
const {readFileSync} = require("node:fs");
const {join} = require("node:path");

function harness() {
  class Element {
    constructor() { this.value = ""; this.textContent = ""; this.hidden = false;
      this.open = false; this.disabled = false; this.children = []; this.events = {};
      this.options = [{value: "", cloneNode() {return this;}}]; }
    addEventListener(event, callback) { (this.events[event] ||= []).push(callback); }
    fire(event, extra = {}) { for (const callback of this.events[event] || [])
      callback({target: this, currentTarget: this, preventDefault() {}, ...extra}); }
    setAttribute() {} removeAttribute() {} appendChild(child) {this.children.push(child);}
    append(...children) {this.children.push(...children);}
    replaceChildren(...children) {this.children = children;}
    add(option) {this.options.push(option);} querySelector() {return this;}
    closest() {return null;} focus() {this.focused = true;}
    showModal() {this.open = true;} close() {this.open = false; this.fire("close");}
  }
  const elements = new Map(), data = new Map(), downloads = [], windowEvents = {};
  const el = id => {if (!elements.has(id)) elements.set(id, new Element()); return elements.get(id);};
  const window = {localStorage: {get length() {return data.size;}, key: i => [...data.keys()][i] ?? null,
    getItem: k => data.get(k) ?? null, setItem: (k, v) => data.set(k, v)},
    addEventListener(event, callback) { (windowEvents[event] ||= []).push(callback); },
    confirm: () => true};
  const context = {window, document: {getElementById: el, createElement: () => new Element(),
    body: {classList: {add() {}, remove() {}}}}, location: {href: "http://localhost/", search: ""},
    history: {replaceState() {}}, URL, Blob, setTimeout, console,
    Option: class {constructor(text, value) {this.text = text; this.value = value;}},
    applyFilters() {}, renderGrid() {}, thumbVersion: () => "1", favLabel: v => v ? "Remove favourite" : "Add favourite",
    toggleFavorite(id) {context.userStore.set("favorite", id, !context.userStore.get("favorite", id));},
    moveCursor(at) {context.cursor = at;}, savedDataError() {}};
  for (const name of ["user-store", "discovery", "dedup", "filters", "prefs", "agents", "catalogue", "kinds", "view-state"])
    Object.assign(context, require(`../../viewer/${name}.js`));
  context.userStore = context.createUserStore(window);
  context.state = {albums: [], filtered: [], albumAliases: {}, favorites: new Set(),
    view: {filters: context.defaultFilters(), storedFilters: context.defaultFilters()},
    prefs: context.normalizePrefs({}), index: {sellers: []}, loadedSellers: 0, failedShards: []};
  context.els = {collection: el("collection-filter"), tag: el("tag-filter"),
    seller: el("seller-filter"), exportFavs: el("export-favs"), grid: el("grid")};
  vm.createContext(context);
  vm.runInContext(readFileSync(join(__dirname, "../../viewer/saved-work.js"), "utf8"), context);
  context.downloadWork = text => downloads.push(JSON.parse(text));
  return {context, state: context.state, store: context.userStore, el, downloads, window,
    storageEvent(key) { for (const callback of windowEvents.storage || []) callback({key}); }};
}
const album = id => ({id, seller: "one", title: `Album ${id}`, thumb: "", links: [],
  album_url: `https://one.x.yupoo.com/albums/${id}`, history: [{at: "2026-10-01", after: id}]});
function open(h, ids = ["100", "300"]) {
  h.state.albums = ids.map(album); h.state.filtered = [...h.state.albums]; h.context.openPreview(ids[0]);
}
function replace(h, trusted = true) {
  h.state.albums = [album("200"), album("300")]; h.state.filtered = [...h.state.albums];
  h.state.albumAliases = trusted ? {100: "200"} : {};
  h.context.refreshSavedState(false);
}

test("inherited storage events refresh saved state, retain open drafts and preserve explicit saves", () => {
  const h = harness(); open(h, ["200", "300"]);
  const other = h.context.createUserStore(h.window);
  other.set("note", "100", "Inherited in another tab");
  other.set("favorite", "100", true);
  assert.equal(h.context.inheritDuplicateWork(other, {100: "200"}), 2);
  h.el("preview-notes").value = "Open draft";
  h.storageEvent("lamp.inherited.v1.note.200");
  assert.equal(h.state.favorites.has("200"), true);
  assert.equal(h.el("preview-notes").value, "Open draft");
  h.el("album-preview").close(); h.context.openPreview("200");
  assert.equal(h.el("preview-notes").value, "Inherited in another tab");
  assert.equal(h.el("preview-title").textContent, "Album 200");
  h.el("preview-notes").value = "Explicit preview save";
  h.el("preview-form").fire("submit");
  h.storageEvent("lamp.inherited.v1.note.200");
  assert.equal(h.el("preview-notes").value, "Explicit preview save");
  assert.equal(other.get("note", "200"), "Explicit preview save");
  assert.equal(other.get("note", "100"), "Inherited in another tab");
});

test("clean replacement reconciles display, history, draft and storage target", () => {
  const h = harness(); h.store.set("note", "200", "Destination note"); open(h); replace(h);
  assert.equal(h.el("preview-title").textContent, "Album 200");
  assert.equal(h.el("preview-notes").value, "Destination note");
  h.el("preview-notes").value = "Edited 200"; h.el("preview-form").fire("submit");
  assert.equal(h.store.get("note", "200"), "Edited 200");
  assert.equal(h.store.get("note", "100"), undefined);
  assert.match(h.el("preview-history").children[0].textContent, /200/);
});

test("dirty replacement keeps the draft and blocks mixed-target actions", () => {
  const h = harness(); open(h); h.el("preview-notes").value = "Unsaved original"; replace(h);
  assert.equal(h.el("preview-title").textContent, "Album 100");
  assert.equal(h.el("preview-notes").value, "Unsaved original");
  h.el("preview-form").fire("submit"); h.el("preview-favourite").fire("click");
  h.el("preview-next").fire("click"); h.el("preview-similar").open = true; h.el("preview-similar").fire("toggle");
  assert.equal(h.store.get("note", "100"), undefined); assert.equal(h.store.get("note", "200"), undefined);
  assert.equal(h.store.get("favorite", "100"), undefined);
  assert.equal(h.el("preview-title").textContent, "Album 100");
  assert.equal(h.el("preview-notes").value, "Unsaved original");
});

test("navigation failure leaves the displayed save target unchanged", () => {
  const h = harness(); open(h); h.el("preview-next").fire("click"); replace(h, false);
  h.el("preview-prev").fire("click");
  assert.equal(h.el("preview-title").textContent, "Album 300");
  h.el("preview-notes").value = "For 300"; h.el("preview-form").fire("submit");
  assert.equal(h.store.get("note", "300"), "For 300"); assert.equal(h.store.get("note", "100"), undefined);
});

test("untrusted replacement is unresolved and exports original saved work", () => {
  const h = harness(); h.store.set("note", "100", "Saved original"); open(h); replace(h, false);
  h.el("preview-notes").value = "Draft"; h.el("preview-form").fire("submit");
  assert.equal(h.store.get("note", "100"), "Saved original");
  h.el("export-favs").fire("click");
  assert.equal(h.downloads[0].entries.find(e => e.kind === "note").value, "Saved original");
  assert.deepEqual(h.downloads[0].unresolved_ids, ["100"]);
});

test("transfer keeps explicit destination fields and stages only missing edited fields", () => {
  const h = harness(); h.store.set("note", "200", ""); open(h);
  h.el("preview-notes").value = "Original draft"; h.el("preview-tags").value = "Compare"; replace(h);
  h.el("preview-transfer").fire("click");
  assert.equal(h.el("preview-title").textContent, "Album 200");
  assert.equal(h.el("preview-notes").value, ""); assert.equal(h.el("preview-tags").value, "Compare");
  assert.equal(h.store.get("tags", "200"), undefined);
  h.el("preview-form").fire("submit");
  assert.deepEqual(h.store.get("tags", "200"), ["Compare"]); assert.equal(h.store.get("note", "200"), "");
});

test("failed save preserves unsaved fields and cross-tab values outside the edit", () => {
  const h = harness(); open(h); h.store.set("collection", "100", "Other tab");
  h.el("preview-notes").value = "Draft";
  const set = h.store.set; h.store.set = () => false; h.el("preview-form").fire("submit");
  assert.equal(h.el("preview-notes").value, "Draft");
  h.store.set = set; h.el("preview-form").fire("submit");
  assert.equal(h.store.get("note", "100"), "Draft"); assert.equal(h.store.get("collection", "100"), "Other tab");
});
