"use strict";

// Viewer preferences that are neither filters nor saved album work: the
// agent purchase links open on, whether the preview offers external
// lookups, and the operator's own exchange rate for displayed prices.
// Stored as one guarded blob like the filters; nothing here touches the DOM.

/**
 * @typedef {{agent: string, lookups: boolean, currency: string, rate: string}} Prefs
 */

const PREFS_HELPERS = typeof module !== "undefined"
  ? { ...require("./filters.js"), ...require("./agents.js") }
  : { readStored, writeStored, ACTIVE_AGENTS };

const PREFS_STORAGE_KEY = "lamp.prefs";

/** @type {Prefs} */
const PREF_DEFAULTS = { agent: "", lookups: false, currency: "", rate: "" };

// Coerce whatever was stored into the shape the viewer expects; anything
// unrecognised, including an agent that has since retired, falls back to
// the default rather than being trusted.
/**
 * @param {unknown} raw
 * @returns {Prefs}
 */
function normalizePrefs(raw) {
  const out = { ...PREF_DEFAULTS };
  if (!raw || typeof raw !== "object") return out;
  const value = /** @type {Record<string, unknown>} */ (raw);
  if (typeof value.agent === "string" &&
      PREFS_HELPERS.ACTIVE_AGENTS.some(agent => agent.id === value.agent)) out.agent = value.agent;
  if (value.lookups === true) out.lookups = true;
  if (typeof value.currency === "string" && /^[A-Z]{3}$/.test(value.currency)) out.currency = value.currency;
  if (typeof value.rate === "string" && /^[0-9]+(?:\.[0-9]{1,6})?$/.test(value.rate) && Number(value.rate) > 0) {
    out.rate = value.rate;
  }
  // A rate without a currency, or the reverse, converts nothing.
  if (!out.currency || !out.rate) out.currency = out.rate = "";
  return out;
}

// The stored blob, as written, before normalisation: lets the viewer say
// that a saved agent is no longer offered instead of silently dropping it.
/** @param {import("./filters.js").StorageHost} host @returns {unknown} */
function storedPrefs(host) {
  try { return JSON.parse(PREFS_HELPERS.readStored(host, PREFS_STORAGE_KEY) || "null"); }
  catch { return null; }
}

/** @param {import("./filters.js").StorageHost} host */
function readPrefs(host) {
  return normalizePrefs(storedPrefs(host));
}

/** @param {import("./filters.js").StorageHost} host @param {unknown} prefs */
function writePrefs(host, prefs) {
  return PREFS_HELPERS.writeStored(host, PREFS_STORAGE_KEY, JSON.stringify(normalizePrefs(prefs)));
}

// A CNY price at the operator's rate ("1 CNY = rate currency"), marked as an
// approximation and carrying the currency code, or "" when no rate is set or
// the price is unknown. Intl knows each currency's fraction digits; a code it
// refuses falls back to two decimals.
/**
 * @param {number | null | undefined} price
 * @param {Prefs} prefs
 */
function convertedPrice(price, prefs) {
  if (!price || !prefs.currency || !prefs.rate) return "";
  const amount = price * Number(prefs.rate);
  if (!Number.isFinite(amount)) return "";
  try {
    return "≈" + new Intl.NumberFormat("en", { style: "currency", currency: prefs.currency,
      currencyDisplay: "code" }).format(amount).replace(/\u00a0/g, " ");
  } catch {
    return `≈${prefs.currency} ${amount.toFixed(2)}`;
  }
}

// What the card figure prints: the converted amount when a rate is set,
// otherwise the CNY figure as before.
/**
 * @param {number | null | undefined} price
 * @param {Prefs} prefs
 * @param {(price: number | null | undefined) => string} formatCny
 */
function displayPrice(price, prefs, formatCny) {
  return convertedPrice(price, prefs) || formatCny(price);
}

if (typeof module !== "undefined") {
  module.exports = { PREFS_STORAGE_KEY, PREF_DEFAULTS, normalizePrefs, storedPrefs, readPrefs, writePrefs,
    convertedPrice, displayPrice };
}
