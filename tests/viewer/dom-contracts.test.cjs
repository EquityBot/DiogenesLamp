"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const viewer = path.resolve(__dirname, "../../viewer");

test("controller DOM declarations match the shipped HTML elements", () => {
  const html = fs.readFileSync(path.join(viewer, "index.html"), "utf8");
  const declarations = fs.readFileSync(path.join(viewer, "controller-types.d.ts"), "utf8")
    .split("export interface ViewerElements {")[1].split("\n}")[0];
  const tags = new Map([...html.matchAll(/<([a-z][a-z0-9]*)\b[^>]*\bid="([^"]+)"/g)]
    .map(match => [match[2], match[1]]));
  const elementTags = {HTMLInputElement: "input", HTMLSelectElement: "select",
    HTMLButtonElement: "button", HTMLDivElement: "div", HTMLDialogElement: "dialog",
    HTMLDetailsElement: "details", HTMLImageElement: "img", HTMLUListElement: "ul",
    HTMLTextAreaElement: "textarea", HTMLFormElement: "form", SVGUseElement: "use"};
  const controls = [...declarations.matchAll(/(?:'([^']+)'|(\w+)):\s*(\w+)/g)];
  assert.ok(controls.length > 50, "the complete control map is checked");
  for (const [, quoted, plain, type] of controls) {
    if (plain === "parentElement") continue;
    const id = quoted || plain;
    assert.ok(tags.has(id), `missing declared element: ${id}`);
    if (elementTags[type]) assert.equal(tags.get(id), elementTags[type], `${id}: ${type}`);
  }
  assert.match(html, /<div id="load-progress"[^>]*><div id="load-progress-bar"/,
    "the progress bar has the parent promised by its DOM contract");
});
