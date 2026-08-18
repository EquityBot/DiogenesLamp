"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");
const path = require("node:path");

const { AGE_ANCHORS, estimatePublishedMs, publishedMs } =
  require(path.join(__dirname, "..", "..", "viewer", "age.js"));

const DAY = 86400000;

test("anchors are strictly increasing in both id and time", () => {
  for (let i = 1; i < AGE_ANCHORS.length; i += 1) {
    assert.ok(AGE_ANCHORS[i][0] > AGE_ANCHORS[i - 1][0]);
    assert.ok(AGE_ANCHORS[i][1] > AGE_ANCHORS[i - 1][1]);
  }
});

test("estimatePublishedMs hits anchors exactly and interpolates between", () => {
  for (const [id, ts] of AGE_ANCHORS) {
    assert.equal(estimatePublishedMs(id), ts);
  }
  const [id0, t0] = AGE_ANCHORS[3];
  const [id1, t1] = AGE_ANCHORS[4];
  const mid = estimatePublishedMs((id0 + id1) / 2);
  assert.ok(mid > t0 && mid < t1);
  assert.ok(Math.abs(mid - (t0 + t1) / 2) < DAY);
});

test("estimatePublishedMs clamps below and extrapolates above", () => {
  assert.equal(estimatePublishedMs(1), AGE_ANCHORS[0][1]);
  const last = AGE_ANCHORS[AGE_ANCHORS.length - 1];
  const beyond = estimatePublishedMs(last[0] + 1000000);
  assert.ok(beyond > last[1]);
  assert.ok(beyond < last[1] + 60 * DAY); // sane rate, not runaway
});

test("publishedMs prefers a real scraped date over the estimate", () => {
  const real = publishedMs({ id: "8348553", date_published: "2026-01-15" });
  assert.equal(real, Date.parse("2026-01-15"));
  // malformed date falls back to the id estimate
  const est = publishedMs({ id: "8348553", date_published: "unknown" });
  assert.equal(est, AGE_ANCHORS[0][1]);
  const none = publishedMs({ id: "8348553" });
  assert.equal(none, AGE_ANCHORS[0][1]);
});
