"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");
const { PREFS_STORAGE_KEY, normalizePrefs, storedPrefs, readPrefs, writePrefs, convertedPrice, displayPrice } =
  require("../../viewer/prefs.js");
const { formatPrice } = require("../../viewer/filters.js");

function host(initial = {}, fail = false) {
  const data = { ...initial };
  return { data, localStorage: {
    getItem(key) { if (fail) throw new Error("blocked"); return key in data ? data[key] : null; },
    setItem(key, value) { if (fail) throw new Error("quota"); data[key] = String(value); },
  } };
}

test("normalizePrefs keeps only choices the viewer can honour", () => {
  assert.deepEqual(normalizePrefs(null), { agent: "", lookups: false, currency: "", rate: "" });
  assert.equal(normalizePrefs({ agent: "hoobuy" }).agent, "hoobuy");
  assert.equal(normalizePrefs({ agent: "cnfans" }).agent, "", "a retired agent is not silently kept");
  assert.equal(normalizePrefs({ agent: "nobody" }).agent, "");
  assert.equal(normalizePrefs({ lookups: "yes" }).lookups, false);
  assert.equal(normalizePrefs({ lookups: true }).lookups, true);
  assert.deepEqual(normalizePrefs({ currency: "AUD", rate: "0.21" }), { agent: "", lookups: false, currency: "AUD", rate: "0.21" });
  for (const partial of [{ currency: "AUD" }, { rate: "0.21" }, { currency: "aud", rate: "0.21" },
    { currency: "AUD", rate: "0" }, { currency: "AUD", rate: "-1" }, { currency: "AUD", rate: "1.1234567" },
    { currency: "AUD", rate: 0.21 }]) {
    const out = normalizePrefs(partial);
    assert.equal(out.currency + out.rate, "", JSON.stringify(partial));
  }
});

test("preferences round-trip through guarded storage as JSON and survive a refusal", () => {
  const shared = host();
  assert.equal(writePrefs(shared, { agent: "oopbuy", lookups: true, currency: "AUD", rate: "0.21" }), true);
  assert.deepEqual(readPrefs(shared), { agent: "oopbuy", lookups: true, currency: "AUD", rate: "0.21" });
  assert.deepEqual(storedPrefs(shared), { agent: "oopbuy", lookups: true, currency: "AUD", rate: "0.21" });
  shared.localStorage.setItem(PREFS_STORAGE_KEY, "{bad json");
  assert.deepEqual(readPrefs(shared), normalizePrefs(null));
  const blocked = host({}, true);
  assert.equal(writePrefs(blocked, { agent: "oopbuy" }), false);
  assert.deepEqual(readPrefs(blocked), normalizePrefs(null));
});

test("converted prices are marked approximate and use the currency's own decimals", () => {
  assert.equal(convertedPrice(180, { agent: "", lookups: false, currency: "AUD", rate: "0.21" }), "≈AUD 37.80");
  assert.equal(convertedPrice(180, { agent: "", lookups: false, currency: "JPY", rate: "21" }), "≈JPY 3,780");
  assert.equal(convertedPrice(0, { agent: "", lookups: false, currency: "AUD", rate: "0.21" }), "", "unknown stays unknown");
  assert.equal(convertedPrice(180, normalizePrefs(null)), "");
  assert.equal(displayPrice(180, normalizePrefs(null), formatPrice), "¥180");
  assert.equal(displayPrice(180, { agent: "", lookups: false, currency: "AUD", rate: "0.21" }, formatPrice), "≈AUD 37.80");
});
