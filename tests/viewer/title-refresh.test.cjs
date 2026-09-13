"use strict";

// app.js is the DOM layer and cannot be required, so this suite lifts the two
// title functions out of the shipped source and runs them against a small
// element double. That keeps the check on the real code rather than a copy.

const test = require("node:test");
const assert = require("node:assert");
const fs = require("node:fs");
const path = require("node:path");
const { matchRanges } = require("../../viewer/catalogue.js");

const SOURCE = fs.readFileSync(
  path.join(__dirname, "..", "..", "viewer", "app.js"), "utf8");

function lift(name) {
  const found = SOURCE.match(new RegExp("\\nfunction " + name + "\\([\\s\\S]*?\\n\\}\\n"));
  assert.ok(found, "app.js no longer declares " + name);
  return found[0];
}

const { fillTitle, refreshCardTitle } = new Function(
  "document", "matchRanges",
  lift("fillTitle") + lift("refreshCardTitle") +
  "\nreturn { fillTitle, refreshCardTitle };"
)({ createElement: makeElement }, matchRanges);

function makeElement(tag) {
  return {
    tagName: tag,
    nodes: [""],
    set textContent(value) { this.nodes = [value]; },
    get textContent() {
      return this.nodes.map((n) => (typeof n === "string" ? n : n.textContent)).join("");
    },
    replaceChildren(...nodes) { this.nodes = nodes; },
  };
}

function marksOf(el) {
  return el.nodes.filter((n) => typeof n !== "string" && n.tagName === "mark")
    .map((n) => n.textContent);
}

function makeCardDouble() {
  const span = makeElement("span");
  return { dataset: {}, span, querySelector: () => span };
}

const ALBUM = { title: "jacket grid indigo 20000019" };

test("a fresh card marks the current query", () => {
  const card = makeCardDouble();
  assert.strictEqual(refreshCardTitle(card, ALBUM, "jacket"), true);
  assert.deepStrictEqual(marksOf(card.span), ["jacket"]);
  assert.strictEqual(card.span.textContent, ALBUM.title);
});

test("a reused card drops the earlier query's marks when the query changes", () => {
  // The reported defect: searching "jacket" and then extending to
  // "jacket grid" left reused cards showing only the "jacket" mark, because
  // the title was filled once, when the card was created.
  const card = makeCardDouble();
  refreshCardTitle(card, ALBUM, "jacket");
  assert.strictEqual(refreshCardTitle(card, ALBUM, "grid"), true);
  assert.deepStrictEqual(marksOf(card.span), ["grid"]);
  assert.strictEqual(card.span.textContent, ALBUM.title);
});

test("a reused card marks every term of a widened query", () => {
  const card = makeCardDouble();
  refreshCardTitle(card, ALBUM, "jacket");
  refreshCardTitle(card, ALBUM, "jacket grid");
  assert.deepStrictEqual(marksOf(card.span), ["jacket", "grid"]);
});

test("clearing the query leaves a reused card with no marks", () => {
  const card = makeCardDouble();
  refreshCardTitle(card, ALBUM, "jacket grid");
  assert.strictEqual(refreshCardTitle(card, ALBUM, ""), true);
  assert.deepStrictEqual(marksOf(card.span), []);
  assert.strictEqual(card.span.textContent, ALBUM.title);
});

test("an unchanged query leaves the reused card alone", () => {
  // Scrolling reuses cards at the same query, and that path has to stay free
  // of title work.
  const card = makeCardDouble();
  refreshCardTitle(card, ALBUM, "jacket");
  const drawn = card.span.nodes;
  assert.strictEqual(refreshCardTitle(card, ALBUM, "jacket"), false);
  assert.strictEqual(card.span.nodes, drawn);
});

test("fillTitle writes plain text when the query matches nothing", () => {
  const el = makeElement("span");
  fillTitle(el, ALBUM.title, "sandal");
  assert.deepStrictEqual(marksOf(el), []);
  assert.strictEqual(el.textContent, ALBUM.title);
});
