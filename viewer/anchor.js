"use strict";

// Scroll anchoring for background rebuilds. Pure functions only - no DOM
// access - so the same file runs in the browser (script tag before app.js)
// and under node for tests. app.js supplies the measurements.

// Which album sits at the top of the viewport, and how far into its row.
// Returns null when there is nothing on screen to anchor on.
//
// viewTop arrives raw and can be negative: scrolled above the grid's first
// row (toolbar in view) it would floor to row -1, and the restore would
// then push the user a full row DOWN on every background refresh. The
// index is capped too - a filter that shrank the list mid-scroll would
// otherwise point past the end and the anchor would be silently dropped.
/**
 * @param {number} viewTop
 * @param {number} rowH
 * @param {number} cols
 * @param {number} count
 */
function anchorAt(viewTop, rowH, cols, count) {
  if (count <= 0 || rowH <= 0 || cols <= 0) return null;
  const top = Math.max(0, viewTop);
  const row = Math.floor(top / rowH);
  const index = Math.min(row * cols, count - 1);
  return { index, offset: top - row * rowH };
}

// Where the viewport has to sit for the anchored album, now at newIndex, to
// keep the pixel position it had. Mirrors anchorAt exactly: the row is
// derived the same way, so a rebuild that moved nothing is a no-op.
/**
 * @param {number} newIndex
 * @param {number} offset
 * @param {number} rowH
 * @param {number} cols
 */
function restoredViewTop(newIndex, offset, rowH, cols) {
  return Math.floor(newIndex / cols) * rowH + offset;
}

if (typeof module !== "undefined") {
  module.exports = { anchorAt, restoredViewTop };
}
