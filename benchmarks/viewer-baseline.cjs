"use strict";

// Measurement-only baseline for the pure catalogue path. Node's stable
// perf_hooks clock avoids adding an npm dependency to a project that has none.

const { performance } = require("node:perf_hooks");
const path = require("node:path");

const { matchSearch, sortAlbums } = require(
  path.join(__dirname, "..", "viewer", "filters.js"));
const { dupKey, bannedTitleKeys, collapseDups } = require(
  path.join(__dirname, "..", "viewer", "dedup.js"));

const ALBUM_COUNT = 10_000;
const WARMUPS = 5;
const SAMPLES = 30;

function fabricatedAlbums() {
  const albums = [];
  for (let index = 0; index < ALBUM_COUNT; index += 1) {
    const seller = `seller-${index % 80}`;
    const item = Math.floor(index / 2);
    albums.push({
      id: String(9_000_000 - index),
      seller,
      title: `Fabricated black jacket product ${item} seller ${seller}`,
      price: index % 11 === 0 ? 0 : (index * 37) % 2_000,
      photo_count: (index * 13) % 40,
      published_ms: 2_000_000_000_000 - index * 60_000,
      updated_at: new Date(2_000_000_000_000 - index * 60_000).toISOString(),
      links: [{ platform: "taobao", url: `https://item.taobao.com/item.htm?id=${item}` }],
    });
  }
  return albums;
}

const ALBUMS = fabricatedAlbums();

function exerciseViewer() {
  const annotated = ALBUMS.map((album) => ({ ...album, dup_key: dupKey(album) }));
  const filtered = annotated.filter((album) => matchSearch(album.title, "black jacket"));
  const sorted = sortAlbums(filtered, "price-asc");
  const banned = bannedTitleKeys(annotated);
  const collapsed = collapseDups(sorted, banned);
  return `${collapsed.out.length}:${collapsed.counts.size}`;
}

function mean(values) {
  return values.reduce((total, value) => total + value, 0) / values.length;
}

function percentile50(values) {
  const ordered = values.slice().sort((left, right) => left - right);
  const middle = Math.floor(ordered.length / 2);
  return ordered.length % 2
    ? ordered[middle]
    : (ordered[middle - 1] + ordered[middle]) / 2;
}

function main() {
  const expected = exerciseViewer();
  for (let index = 0; index < WARMUPS; index += 1) exerciseViewer();

  const samples = [];
  for (let index = 0; index < SAMPLES; index += 1) {
    const start = performance.now();
    const result = exerciseViewer();
    samples.push(performance.now() - start);
    if (result !== expected) throw new Error("viewer workload returned an unstable result");
  }

  const average = mean(samples);
  const variance = mean(samples.map((value) => (value - average) ** 2));
  const standardDeviation = Math.sqrt(variance);
  process.stdout.write(`${JSON.stringify({
    node: process.version,
    album_count: ALBUM_COUNT,
    warmups: WARMUPS,
    samples: SAMPLES,
    result: expected,
    median_ms: percentile50(samples),
    mean_ms: average,
    standard_deviation_ms: standardDeviation,
    coefficient_of_variation: standardDeviation / average,
    min_ms: Math.min(...samples),
    max_ms: Math.max(...samples),
    threshold_enabled: false,
  })}\n`);
}

main();
