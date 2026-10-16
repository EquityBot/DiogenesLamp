"use strict";

/**
 * @typedef {"favorite"|"note"|"tags"|"collection"|"review"|"search"|"dismiss"} UserKind
 * @typedef {{kind: UserKind, id: string, value: unknown}} UserEntry
 * @typedef {{favorite: boolean, note: string, tags: string[], collection: string,
 * review: number, search: Record<string, string | boolean> | null, dismiss: boolean}} UserValues
 * @typedef {{localStorage: Pick<Storage, "getItem"|"setItem"|"key"|"length">}} UserHost
 */

const USER_PREFIX = "lamp.user.v1.";
const INHERITED_PREFIX = "lamp.inherited.v1.";
const INHERITED_KINDS = new Set(["favorite", "note", "tags", "collection"]);
const USER_KINDS = new Set(["favorite", "note", "tags", "collection", "review", "search", "dismiss"]);

/** @param {unknown} value @returns {value is Record<string, unknown>} */
function userObject(value) {
  return !!value && typeof value === "object" && !Array.isArray(value);
}

/** @param {UserKind} kind @param {string} id @param {unknown} value */
function validUserEntry(kind, id, value) {
  if (!id || id.length > 200 || !USER_KINDS.has(kind)) return false;
  if (["favorite", "note", "tags", "collection"].includes(kind) && !/^\d{1,24}$/.test(id)) return false;
  if (kind === "favorite" || kind === "dismiss") return typeof value === "boolean";
  if (kind === "note") return typeof value === "string" && value.length <= 4000;
  if (kind === "collection") return typeof value === "string" && value.length <= 80;
  if (kind === "tags") return Array.isArray(value) && value.length <= 20 &&
    value.every(tag => typeof tag === "string" && tag.length <= 50);
  if (kind === "review") return typeof value === "number" && Number.isFinite(value) && value >= 0;
  if (kind === "search") return value === null || userObject(value) &&
    Object.keys(value).length <= 24 &&
    Object.values(value).every(v => typeof v === "boolean" || typeof v === "string" && v.length <= 2000);
  return false;
}

/**
 * Every field has its own key. Updating one saved album cannot replace
 * another tab's changes to a different album. Explicit same-field writes are
 * last-write-wins; automatic inheritance uses lower-priority persistent keys.
 * The host's storage getter is always reached inside a guard.
 * @param {UserHost} host
 * @param {(message: string) => void} [onError]
 */
function createUserStore(host, onError = () => {}) {
  /** @type {Map<string, UserEntry | undefined>} */
  let cached = new Map();
  /** @param {UserKind} kind @param {string} id */
  const keyFor = (kind, id) => USER_PREFIX + kind + "." + encodeURIComponent(id);
  /** @param {unknown} error */
  function fail(error) {
    onError("Saved data could not be read or written. Keep this page open and export a backup. " +
      (error instanceof Error ? error.message : ""));
  }
  function effectiveEntries() {
    /** @type {Map<string, UserEntry>} */
    const entries = new Map();
    for (const [key, entry] of cached) {
      if (!entry) continue;
      const explicitKey = keyFor(entry.kind, entry.id);
      // Even an unreadable explicit entry blocks an inherited fallback.
      if (key.startsWith(INHERITED_PREFIX) && cached.has(explicitKey)) continue;
      entries.set(explicitKey, entry);
    }
    return [...entries.values()];
  }
  function load() {
    try {
      const storage = host.localStorage;
      /** @type {Map<string, UserEntry | undefined>} */
      const entries = new Map();
      for (let i = 0; i < storage.length; i++) {
        const key = storage.key(i);
        if (!key) continue;
        const prefix = key.startsWith(USER_PREFIX) ? USER_PREFIX :
          key.startsWith(INHERITED_PREFIX) ? INHERITED_PREFIX : null;
        if (!prefix) continue;
        const split = key.indexOf(".", prefix.length);
        if (split < 0) continue;
        try {
          const kind = /** @type {UserKind} */ (key.slice(prefix.length, split));
          const id = decodeURIComponent(key.slice(split + 1));
          const value = JSON.parse(storage.getItem(key) || "null");
          if (!validUserEntry(kind, id, value) ||
              prefix === INHERITED_PREFIX && !INHERITED_KINDS.has(kind)) {
            throw new Error("An invalid saved entry needs recovery.");
          }
          entries.set(key, { kind, id, value });
        } catch (error) {
          fail(error);
          entries.set(key, cached.get(key));
        }
      }
      cached = entries;
    } catch (error) { fail(error); }
    return effectiveEntries();
  }
  /** @param {string} prefix @param {UserKind} kind @param {string} id @param {unknown} value */
  function write(prefix, kind, id, value) {
    if (!validUserEntry(kind, id, value) ||
        prefix === INHERITED_PREFIX && !INHERITED_KINDS.has(kind)) {
      onError("The saved value is invalid or too long.");
      return false;
    }
    const key = prefix + kind + "." + encodeURIComponent(id);
    try {
      host.localStorage.setItem(key, JSON.stringify(value));
      cached.set(key, { kind, id, value });
      return true;
    } catch (error) { fail(error); return false; }
  }
  /** @param {UserKind} kind @param {string} id @param {unknown} value */
  const set = (kind, id, value) => write(USER_PREFIX, kind, id, value);
  /** @param {UserKind} kind @param {string} id @param {unknown} value */
  const setInherited = (kind, id, value) => write(INHERITED_PREFIX, kind, id, value);
  /** @template {UserKind} K @param {K} kind @param {string} id
   * @returns {UserValues[K] | undefined} */
  function get(kind, id) {
    let key = keyFor(kind, id);
    try {
      const storage = host.localStorage;
      let raw = storage.getItem(key);
      if (raw === null && INHERITED_KINDS.has(kind)) {
        key = INHERITED_PREFIX + kind + "." + encodeURIComponent(id);
        raw = storage.getItem(key);
      }
      if (raw !== null) {
        const value = JSON.parse(raw);
        if (validUserEntry(kind, id, value)) return value;
        throw new Error("An invalid saved entry needs recovery.");
      }
      return undefined;
    } catch (error) { fail(error); return /** @type {UserValues[K] | undefined} */ (cached.get(key)?.value); }
  }
  /** @param {UserKind} kind @param {string} id */
  function has(kind, id) {
    // Failed parsing or reading cannot establish that saved data is absent.
    try {
      const storage = host.localStorage;
      return storage.getItem(keyFor(kind, id)) !== null || INHERITED_KINDS.has(kind) &&
        storage.getItem(INHERITED_PREFIX + kind + "." + encodeURIComponent(id)) !== null;
    }
    catch (error) { fail(error); throw error; }
  }
  function migrate() {
    try {
      const storage = host.localStorage;
      if (storage.getItem("lamp.user-migrated.v1")) return;
      for (const key of ["lamp.favorites", "candle.favorites"]) {
        const raw = storage.getItem(key);
        if (!raw) continue;
        const ids = JSON.parse(raw);
        if (!Array.isArray(ids)) continue;
        for (const value of ids) {
          const id = String(value);
          if (!validUserEntry("favorite", id, true)) continue;
          // A false value is a deliberate removal, including after a partial migration.
          if (!has("favorite", id) && !set("favorite", id, true)) return;
        }
        break;
      }
      storage.setItem("lamp.user-migrated.v1", "1");
    } catch (error) { fail(error); }
  }
  function favorites() {
    return new Set(load().filter(entry => entry.kind === "favorite" && entry.value === true)
      .map(entry => entry.id));
  }
  /** @template {UserKind} K @param {K} kind @returns {Map<string, UserValues[K]>} */
  function section(kind) {
    return new Map(load().filter(entry => entry.kind === kind).map(entry => [entry.id, /** @type {UserValues[K]} */ (entry.value)]));
  }
  migrate();
  load();
  return { load, get, has, set, setInherited, favorites, section };
}

/** @param {unknown} raw @returns {UserEntry[]} */
function parseUserBackup(raw) {
  if (!userObject(raw)) throw new Error("Choose a DiogenesLamp JSON backup.");
  /** @type {UserEntry[]} */
  let entries;
  if (raw.format === "diogeneslamp" && raw.version === 1 && Array.isArray(raw.entries)) {
    entries = raw.entries;
  } else if (!raw.format && Array.isArray(raw.albums)) {
    // The earlier favourites export has metadata but no restorable storage entries.
    entries = raw.albums.map(album => ({ kind: "favorite", id: String(album.id), value: true }));
  } else {
    throw new Error("This backup format or version is not supported.");
  }
  if (entries.length > 100000) throw new Error("The backup contains too many saved entries.");
  const keys = new Set();
  for (const entry of entries) {
    if (!userObject(entry) || typeof entry.kind !== "string" || typeof entry.id !== "string" ||
        !validUserEntry(/** @type {UserKind} */ (entry.kind), entry.id, entry.value)) {
      throw new Error("The backup contains an invalid saved entry. Nothing was imported.");
    }
    const key = entry.kind + "." + entry.id;
    if (keys.has(key)) throw new Error("The backup repeats a saved entry. Nothing was imported.");
    keys.add(key);
  }
  return entries;
}

/**
 * Validate the entire file before writing. Existing fields win, so importing
 * twice is harmless and an old backup cannot erase newer notes or removals.
 * @param {ReturnType<typeof createUserStore>} store
 * @param {unknown} raw
 */
function importUserBackup(store, raw) {
  const entries = parseUserBackup(raw);
  let imported = 0;
  try {
    for (const { kind, id, value } of entries) {
      if (store.has(kind, id)) continue;
      if (!store.set(kind, id, value)) throw new Error("Storage write failed.");
      imported++;
    }
  } catch {
    throw new Error("Import stopped because storage is unavailable. Existing data was preserved; " +
      imported + " entries were imported. Keep the backup and try again.");
  }
  return imported;
}

/**
 * Copy saved fields to the newest listing only when that field has no entry.
 * Retain originals for backup; an explicit false or empty value wins.
 * @param {ReturnType<typeof createUserStore>} store
 * @param {Record<string, string>} aliases
 */
function inheritDuplicateWork(store, aliases) {
  const entries = store.load().filter(entry =>
    ["favorite", "note", "tags", "collection"].includes(entry.kind) &&
    /^[0-9]{1,24}$/.test(aliases[entry.id] || "") && aliases[entry.id] !== entry.id);
  // When several older listings have work, prefer the higher album ID.
  entries.sort((a, b) => BigInt(a.id) > BigInt(b.id) ? -1 : a.id === b.id ? 0 : 1);
  let copied = 0;
  try {
    for (const entry of entries) {
      const newest = aliases[entry.id];
      if (store.has(entry.kind, newest)) continue;
      if (!store.setInherited(entry.kind, newest, entry.value)) break;
      copied++;
    }
  } catch { /* The store reports unavailable storage and keeps the originals. */ }
  return copied;
}

if (typeof module !== "undefined") {
  module.exports = { USER_PREFIX, INHERITED_PREFIX, validUserEntry, createUserStore, parseUserBackup, importUserBackup,
    inheritDuplicateWork };
}
