"use strict";

/**
 * @typedef {"favorite"|"note"|"tags"|"collection"|"review"|"search"|"dismiss"} UserKind
 * @typedef {{kind: UserKind, id: string, value: unknown}} UserEntry
 * @typedef {{localStorage: Pick<Storage, "getItem"|"setItem"|"key"|"length">}} UserHost
 */

const USER_PREFIX = "lamp.user.v1.";
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
 * another tab's changes to a different album. Same-field writes are last-write-wins.
 * The host's storage getter is always reached inside a guard.
 * @param {UserHost} host
 * @param {(message: string) => void} [onError]
 */
function createUserStore(host, onError = () => {}) {
  /** @type {Map<string, UserEntry>} */
  let cached = new Map();
  /** @param {UserKind} kind @param {string} id */
  const keyFor = (kind, id) => USER_PREFIX + kind + "." + encodeURIComponent(id);
  /** @param {unknown} error */
  function fail(error) {
    onError("Saved data could not be read or written. Keep this page open and export a backup. " +
      (error instanceof Error ? error.message : ""));
  }
  function load() {
    try {
      const storage = host.localStorage;
      /** @type {Map<string, UserEntry>} */
      const entries = new Map();
      for (let i = 0; i < storage.length; i++) {
        const key = storage.key(i);
        if (!key || !key.startsWith(USER_PREFIX)) continue;
        const split = key.indexOf(".", USER_PREFIX.length);
        if (split < 0) continue;
        try {
          const kind = /** @type {UserKind} */ (key.slice(USER_PREFIX.length, split));
          const id = decodeURIComponent(key.slice(split + 1));
          const value = JSON.parse(storage.getItem(key) || "null");
          if (!validUserEntry(kind, id, value)) throw new Error("An invalid saved entry needs recovery.");
          entries.set(key, { kind, id, value });
        } catch (error) {
          fail(error);
          const previous = cached.get(key);
          if (previous) entries.set(key, previous);
        }
      }
      cached = entries;
    } catch (error) { fail(error); }
    return [...cached.values()];
  }
  /** @param {UserKind} kind @param {string} id @param {unknown} value */
  function set(kind, id, value) {
    if (!validUserEntry(kind, id, value)) {
      onError("The saved value is invalid or too long.");
      return false;
    }
    const key = keyFor(kind, id);
    try {
      host.localStorage.setItem(key, JSON.stringify(value));
      cached.set(key, { kind, id, value });
      return true;
    } catch (error) { fail(error); return false; }
  }
  /** @param {UserKind} kind @param {string} id */
  function get(kind, id) {
    const key = keyFor(kind, id);
    try {
      const raw = host.localStorage.getItem(key);
      if (raw !== null) {
        const value = JSON.parse(raw);
        if (validUserEntry(kind, id, value)) return value;
        throw new Error("An invalid saved entry needs recovery.");
      }
      return undefined;
    } catch (error) { fail(error); return cached.get(key)?.value; }
  }
  /** @param {UserKind} kind @param {string} id */
  function has(kind, id) {
    // Failed parsing or reading cannot establish that saved data is absent.
    try { return host.localStorage.getItem(keyFor(kind, id)) !== null; }
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
          if (storage.getItem(keyFor("favorite", id)) === null && !set("favorite", id, true)) return;
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
  /** @param {UserKind} kind */
  function section(kind) {
    return new Map(load().filter(entry => entry.kind === kind).map(entry => [entry.id, entry.value]));
  }
  migrate();
  load();
  return { load, get, has, set, favorites, section };
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

if (typeof module !== "undefined") {
  module.exports = { USER_PREFIX, validUserEntry, createUserStore, parseUserBackup, importUserBackup };
}
