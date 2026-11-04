// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: Apache-2.0

"use strict";

// app.js is the DOM layer and cannot be required, so this suite lifts
// renderShops out of the shipped source, as title-refresh.test.cjs does.

const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const SOURCE = fs.readFileSync(
  path.join(__dirname, "..", "..", "viewer", "app.js"), "utf8");

function element(tag) {
  return { tagName: tag, children: [], addEventListener() {},
    appendChild(child) { this.children.push(child); },
    replaceChildren(...children) { this.children = children; } };
}

test("shop groups are headed in platform-name order", () => {
  const found = SOURCE.match(/\nfunction renderShops\([\s\S]*?\n\}\n/);
  assert.ok(found, "app.js no longer declares renderShops");
  const panel = element("div");
  const renderShops = new Function("document", "els", "setShopsOpen",
    found[0] + "\nreturn renderShops;")({ createElement: element }, { shopsPanel: panel }, () => {});
  renderShops([
    { platform: "taobao global", name: "B", url: "https://b.example" },
    { platform: "taobao", name: "A", url: "https://a.example" },
    { platform: "1688", name: "C", url: "https://c.example" },
  ]);
  const headings = panel.children.filter(c => c.tagName === "h3").map(c => c.textContent);
  assert.deepEqual(headings, ["1688", "taobao", "taobao global"]);
});
