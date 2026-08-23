# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

## Users

One operator at a desk, browsing reseller catalogues they already follow. They
know the sellers by name, they are hunting for a specific item or scanning what
is new since the last scrape, and they decide in seconds per card whether to
open the original album. Sessions are long and scroll-heavy: catalogues run to
tens of thousands of albums across sellers.

Inferred from the repository (README, viewer behaviour, demo data generator);
not confirmed in interview.

## Product Purpose

Yupoo seller catalogues paginate every 20 or so albums and load slowly. The
project scrapes album covers, titles and purchase links into a local index
once, then serves the whole catalogue, across every seller, as one page that
scrolls, searches and filters instantly. Success is that the operator finds the
item faster locally than they could on Yupoo, and never waits for a page.

## Positioning

The index is local and cross-seller. Yupoo itself can only ever show one
seller, twenty at a time, at network speed. Nothing about a single seller page
can offer corpus-wide search, cross-seller duplicate collapsing, or an age
filter that knows an album's date without fetching it.

## Operating Context

Command line first, browser second. The operator runs `lamp import` on a link
dump, `lamp scrape` (slow first run, incremental after), then `lamp serve`,
which opens `http://localhost:<port>`. Data lives in `data/` beside the clone:
`index.json`, per-seller shards, and cover thumbnails. Sellers can be skipped
in `sources.toml`. A scrape can leave a seller with no shard on disk, so the
viewer routinely loads a partially available catalogue.

## Capabilities and Constraints

- Viewer is dependency-free vanilla JS with no build step and no bundler.
  Editing `viewer/*.js` and reloading is the whole development loop.
- Served by a Python standard-library HTTP server out of the source tree. No
  CDN, no external fonts, no network at view time. Everything ships in-repo.
- The grid is virtualised: only rows near the viewport exist in the DOM, card
  geometry is read back out of CSS custom properties, and scroll position is
  anchored across background shard loads. Any redesign must keep fixed card
  geometry readable from `:root`.
- Catalogue loads stream: shards arrive concurrently and the UI refreshes at
  most every 200 ms while the operator is already scrolling.
- Favourites, filter selection and colour theme persist in `localStorage`.
- Existing filters: title search, seller, category, posting age, new since last
  scrape, favourites, hide duplicates. Duplicate groups open as a peek view.
- Python 3.11 or newer. Viewer logic modules are tested with `node --test`,
  everything else with `pytest`.
- Covers and metadata only. Detail photos are never downloaded; every card
  links back to Yupoo.

## Brand Commitments

Name: DiogenesLamp. The lamp reference is the product's one piece of
personality, after the story of Diogenes carrying a lamp in daylight looking
for an honest man. Voice in the README and in the UI is plain, specific and
unhurried, with no marketing register.

## Evidence on Hand

- `tests/make_demo_data.py` writes 5 sellers by 2000 albums with generated
  gradient covers for the first 300 of each, so every viewer state can be seen
  without scraping anything.
- `docs/viewer.webp` is the current README screenshot, generated from that
  demo data.
- Real catalogue data is the operator's own and stays off the repository.
- No usage numbers, no testimonials, no user research. Future work must not
  invent any.

## Product Principles

1. The corpus is the unit. Every control acts across all sellers at once; that
   is the only thing Yupoo cannot do.
2. Scanning beats reading. The operator judges a card by its cover in under a
   second, so nothing may compete with the cover for attention.
3. Never lie about state. A blank grid names which blank it is: still loading,
   nothing matched, or nothing scraped yet. A missing seller is named, not
   silently dropped.
4. Speed is a feature, not an optimisation. Anything that costs a frame during
   scroll or a re-decode of an already loaded cover is a defect.
5. Keyboard and screen reader parity is not optional. Every control is
   labelled and reachable, focus survives virtualised re-renders, and reduced
   motion is honoured.

## Accessibility & Inclusion

No external standard was mandated, but the implementation already commits to:
visible focus rings on `:focus-visible` in both themes, hit areas at or above
24 px, state never carried by colour alone, `aria-busy` while the catalogue
streams, `aria-setsize`/`aria-posinset` on virtualised cards, focus rescue when
a focused card is recycled away, a skip link past the toolbar, Escape closing
overlays, and `prefers-reduced-motion` support. Treat all of it as binding.
