<p align="center">
  <picture>
    <source media="(max-width: 600px) and (prefers-color-scheme: light)" srcset=".github/assets/banner-phone-light.svg">
    <source media="(max-width: 600px)" srcset=".github/assets/banner-phone.svg">
    <source media="(prefers-color-scheme: light)" srcset=".github/assets/banner-light.svg">
    <img src=".github/assets/banner.svg" width="100%" alt="DiogenesLamp. Every Yupoo catalogue on one searchable page. Pixel art of a lit clay oil lamp beside the DiogenesLamp wordmark in slab-serif block letters.">
  </picture>
</p>

<div align="center">

[![ci](https://img.shields.io/github/actions/workflow/status/EquityBot/DiogenesLamp/ci.yml?branch=main&label=ci&labelColor=000000&style=flat)](https://github.com/EquityBot/DiogenesLamp/actions/workflows/ci.yml)
[![rust: 1.99+](https://img.shields.io/badge/rust-1.99%2B-66023C?labelColor=000000&style=flat&color=66023C)](https://www.rust-lang.org/)
[![licence: Apache-2.0](https://img.shields.io/badge/licence-Apache--2.0-66023C?labelColor=000000&style=flat&color=66023C)](LICENSE)

</div>

Browse any Yupoo catalogue locally as one fast, searchable, infinite-scroll page.

Yupoo seller catalogues paginate every 20-ish albums and load slowly. DiogenesLamp
scrapes the album covers, titles, and any taobao/tmall/weidian purchase links into
a local index once, then serves a single page where the whole catalogue - across all
your sellers - scrolls, searches, and filters locally. Clicking an item opens the
original Yupoo album for the detail photos.

On this page:

- [Install](#install)
- [Use](#use)
- [Data](#data)
- [Diagnostics and recovery](#diagnostics-and-recovery)
- [Cached covers and visual suggestions](#cached-covers-and-visual-suggestions)
- [Politeness and scope](#politeness-and-scope)
- [Development](#development)

## Install

Requires a Rust toolchain (the version is pinned in `rust-toolchain.toml`).
`lamp` is a single binary with the viewer assets built in:

    git clone https://github.com/EquityBot/DiogenesLamp
    cd DiogenesLamp
    cargo install --locked --path .

No tagged release is required to install from the clone, and the installed
binary works outside the source tree. During development, `cargo run --` runs
the current source; viewer edits take effect at the next build.

## Use

1. Put your Yupoo links in a text or CSV file, one URL per line (messy exports
   are fine - duplicates, page URLs, and non-Yupoo links are handled). Rows
   like `12,https://seller.x.yupoo.com` work too: a leading numeric index
   (comma- or tab-separated) is stripped, and the rest of the line is taken
   as the URL, so URLs that themselves contain commas survive intact.

       lamp import links.csv

   This creates or updates `sources.toml`. Non-Yupoo shop links (taobao,
   weidian, link aggregators) are kept as a link-out directory.

   To leave a seller out of scrapes - a catalogue too large to be worth the
   hours, say - add `skip = true` to its entry. Commenting it out would not
   last, since `lamp import` rewrites the file from your dump.

2. Scrape (the first run is slow by design - it rate-limits itself to stay
   polite; re-runs only fetch what changed):

       lamp scrape

   Big catalogues are mostly stale stock. Skipping listings older than 6
   months typically removes about two thirds of the work:

       lamp scrape --max-age-days 180

   Ages come from the album page's publish date, or from the album id where
   no date was captured (ids are sequential, so they date an album without
   costing a request - which is what lets the scrape skip a listing before
   fetching it). The flag decides what to fetch, not what to keep: albums
   already scraped stay in `data/`, and the viewer's age filter hides them.

   Other flags: `--seller <id>` for one seller, `--full` to re-fetch
   everything, `--dry-run` to count list-page work without fetching album
   pages or writing files, `--workers N` (default 8), `--rps N` (default 8.0, global
   request rate cap). Raising `--rps` alone does little: each seller host is
   held to 1 request/second, so more hosts have to be in flight for a higher
   cap to matter. Raise `--workers` with it.

   Incremental re-runs skip unchanged titles, but records written before
   price extraction (no `price` field on disk) are fetched once so catalogue
   prices can backfill. A re-fetched album keeps the cover already on disk
   unless its cover URL changed, and a cover missing from disk is fetched on
   its own without the album page. Use `--full` to re-fetch everything,
   covers included.

   Each seller's result line prints as soon as that seller finishes. Ctrl+C
   stops the run: sellers not yet started are dropped, running ones save what
   they have, and `index.json` is still written. Press it again to abandon
   the running sellers.

   To revisit unchanged album details, opt into a freshness budget:

       lamp scrape --refresh-details-days 14 --refresh-limit 100

   This permits up to 100 extra detail requests across the run, oldest checked
   records first within each seller. It preserves the normal request limits.
   Changed and new listings still follow the ordinary scrape policy. Use the
   same flags with `--dry-run` to count work; dry runs fetch listing pages but
   do not fetch details or write files. `--retry-failed` targets IDs recorded
   as failed in earlier runs, subject to the listing and age limit. Seller
   summaries persist in `data/runs/`, including separate listing, attempt and
   successful-run timestamps.

   Only one writer can use a data directory at a time. A competing scrape or
   maintenance command exits with an explanation. The OS releases the lock
   after a crash; the `.lamp.lock` file may remain and should stay in place.

   An album that has disappeared from a seller's listing is kept, marked
   gone, and available under Favourites, a collection or Changed since review. It comes back
   if the seller lists it again.

3. Browse:

       lamp serve

   `--port N` changes the port and `--no-browser` stops it opening one.
   Global options go *before* the subcommand: `lamp --data other-dir serve`
   and `lamp --sources other.toml scrape`.

   Search titles and seller IDs, or paste an exact album ID, marketplace item
   ID, Yupoo album URL or purchase link. A pasted shopping-agent link
   (KakoBuy, Hoobuy, Oopbuy, Mulebuy, Joyagoo, CSSBuy, Sugargoo, Superbuy,
   LoongBuy, Pikobuy, iTaobuy, BBDbuy, Ootdbuy, Lovegobuy, Cnshopper, Alleurobuy,
   RizzitGO, Vigorbuy, USFans, Litbuy, hipobuy, GTBuy, and the retired CNFans,
   AllChinaBuy and Pandabuy) is
   unwrapped to the taobao, tmall, weidian or 1688 item it carries, the same
   way the scraper stores such links from album text. Stored descriptions are
   searched after titles and named as the match reason; the panel under
   **More filters and saved work** says how many loaded albums have one
   recorded. Press `/` to focus search.
   Filter by seller, category or posting age, and sort by price, photo count or
   newest first. **More filters and saved work** includes minimum and maximum
   CNY prices, a separate unknown-price toggle, collections, tags and review
   filters. Bounds include their endpoints. Prices come from extracted text;
   they are not a guarantee of a current sale price.

   Click a title or press `p` on a card for its local preview. It shows the
   cached cover, full title, price extraction evidence, timestamps, purchase
   links and the last 50 recorded price, link or availability changes. Unknown
   extraction is labelled separately from a price drop. Arrow Left/Right moves
   between previews; Escape closes it and returns focus to the catalogue.
   Editing a field disables those arrow shortcuts while typing. The cover
   and Enter on the card still open the original Yupoo album for detail photos.

   The folded corner saves a favourite. The preview can save notes, up to 20
   tags and one named collection per album. Use a collection such as Compare
   to keep a shortlist visible together. Choose a loaded seller and **Mark
   seller reviewed** to record the catalogue's timestamp. **Added since review**
   uses first-seen dates; **Changed since review** uses recorded change dates.

   **Back up saved work** exports a versioned JSON file with favourites, notes,
   tags, collections, review checkpoints, saved searches and dismissed visual
   suggestions. It includes saved IDs whose seller data is unavailable, even
   during loading. Loaded album metadata is supplementary and labelled partial
   when appropriate. Import this file under **More filters and saved work**.
   Imports add missing fields and keep existing values, including deliberate
   favourite removals and unreadable entries. A storage read failure stops
   import instead of treating the field as empty. Repeating an import is harmless. Earlier favourites
   exports can also be imported. Files are limited to 20 MB and 100,000 entries.

   Saved work stays in this browser and origin, including the port. Different
   albums and fields update independently across tabs; the last save wins if
   both tabs edit the same field. Storage failures appear on screen. Keep a
   downloaded backup before clearing browser data or changing ports. Filter
   URLs and named searches restore the current view; Back restores earlier
   filters. **Export current results as CSV** exports the visible result set,
   with catalogue completeness and filter scope recorded in every row, plus
   the stored description, the title-read kind and, when an agent is chosen,
   a rebuilt agent link for every marketplace item.

   **Cross-reference a spreadsheet** takes a community finds sheet (CSV or
   TSV, comma, semicolon or tab separated, up to 20 MB and 100,000 rows),
   reads every marketplace and agent link in it, matches them to albums by
   item and tags the matches `sheet:<file name>`; the Tag filter then shows
   which followed sellers carry them. Tags are additive, so nothing saved is
   overwritten. The status line gives rows, links, distinct items, matches
   and items not found in the loaded catalogue.

   **Kind, read from titles (experimental)** groups albums across sellers by
   a garment kind guessed from title keywords in English and Chinese: shoes,
   jackets, hoodies, tees, pants, shorts, sets, bags, watches, jewellery,
   accessories. A title with no keyword is Unsorted, and a part such as a
   watch strap or shoelaces is an accessory. Precision on real catalogues is
   unmeasured; the seller's own category filter is unchanged.

   **Links and prices** holds three choices that stay in this browser:

   - **Open purchase links on** an agent. Card marks keep their platform name
     and open on the agent; the preview shows the marketplace link and the
     agent link side by side. Links are rebuilt from the item id with no
     referral codes, from formats read from the public converters on
     27 September 2026 (`viewer/agents.js` records the date). An agent that
     shuts down leaves the menu but its old links still resolve. Agents that
     name the marketplace by a channel code have none for tmall, so a tmall
     item keeps its marketplace link only; the agents that wrap the whole
     URL carry it.
   - **QC photo lookups** add a link per marketplace item in the preview to
     rep.tools' QC finder. Off by default: following one sends the item link
     to rep.tools, which may keep it. Nothing is fetched until you click.
   - **Show prices in** a currency at your own rate, entered as 1 CNY = rate.
     The card figure prints the converted amount marked ≈; the preview keeps
     the CNY figure and the rate. It is your estimate before fees and
     shipping, not a quote. Sorting, price bounds and CSV stay in CNY.

   The grid is a keyboard surface. Tab reaches one card - the one under the
   cursor, whose cover link, favourite, preview and purchase links follow it, while
   every other card is skipped - then the arrow keys move that cursor from
   cover to cover (Home, End, Page Up and Page Down included), Enter opens the
   album under the cursor, and `f` keeps it. Ctrl/Cmd and Alt combinations keep
   their browser actions, including Find, Print and Back. The cursor scrolls the grid to
   follow itself, so it works the same at album 12 and album 40,000.

   The theme button cycles system, light and dark. Every control is labelled,
   Escape closes the preview or shops panel, and a skip link jumps
   past the head band to the grid.

   The index keeps the newest duplicate listing before any filters apply.
   A shared marketplace item identifies duplicates across sellers; a specific
   title can identify a repeated posting within one seller. Short generic titles
   and titles used more than twice within a seller remain separate. Posting date
   wins, using the album ID estimate when no valid date is recorded; the higher
   album ID breaks date ties. **Age** hides listings older than 30/90/180/365 days.

   Favourites, notes, tags and collections on an older duplicate copy to the
   newest listing when its field has no saved entry. Existing values, including
   an empty note or a removed favourite, win. If several older listings have a
   field, the highest album ID supplies it. Originals remain in saved-work backups.
   Copies wait for an admitted destination and stable duplicate membership.
   Older indexes need every seller shard to load successfully before new copies
   are saved; run `lamp reindex` to rebuild their selection contract.

   If a listing changes while its preview is open, a clean form follows its
   confirmed replacement. An edited form keeps its draft and pauses saving and
   navigation. Transfer stages edited fields missing from the latest listing;
   existing saved values remain visible for review. A retained unsaved draft
   lasts only while that page stays open and is excluded from saved-work backups.

## Data

Fetched data lives in `data/` (gitignored): `index.json`, seller shards in
`catalog/`, cached covers in `thumbs/` and run summaries in `runs/`. The seller
list stays in the gitignored `sources.toml`. Browser annotations are separate;
use **Back up saved work** to preserve them.

Metadata uses schema version 1. Existing unversioned shards remain readable.
Unsupported versions, malformed records and unsafe paths fail visibly.

New indexes list each seller's selected `album_ids` and map older IDs to the
selected IDs in `album_aliases`. Raw shards retain captures for refresh, history
and recovery. To rebuild an existing catalogue locally, run `lamp reindex` (or
`lamp --data path/to/data reindex`); it fetches nothing and preserves the scrape
timestamp. The viewer also selects the newest listings while loading an older
index. A failed selected shard does not make its older duplicates reappear.
Rebuilt indexes carry `selection_version: 1`. Canonical selection excludes
records whose links fail HTTPS, hostname or credential admission, preserving
the original captures. `doctor` reports their count as `unadmitted`.
Reindex can recover valid selected records beside excluded historical records;
missing, corrupt or ambiguous shards prevent replacement of the index.

Each album fetched since descriptions were kept carries `description`, the
album text cut at 500 characters (`description_truncated` marks a cut), after
the whole text has been read for links and prices. Records written earlier
have no such field; they gain one when their detail page is next fetched,
which the refresh budget does oldest first, so coverage grows with each
budgeted run. `--full` fetches everything, covers included, and is not a
description-only operation.

### Diagnostics and recovery

    lamp --data data doctor
    lamp --data data snapshot
    lamp --data data restore data/snapshots/<snapshot-name>.zip

`doctor` reads files without changing them. It reports invalid or missing
metadata, duplicate IDs, cover coverage, unknown prices, missing detail dates,
albums without a recorded description and recorded detail failures. It returns a non-zero exit code for structural
errors. It does not verify whether a remote listing or extracted price is current.

`snapshot` saves metadata only, excluding images, browser annotations, source
lists and debug captures. It requires every indexed shard to exist and pass
validation before creating an archive. Each archive is limited to 10,000 files and 256 MB
of uncompressed metadata. Snapshots accumulate in `data/snapshots/`; retain the
ones you need. Restore validates the whole archive before writing, preserves
the previous metadata in another archive and replaces the index last. Files
replace atomically one at a time; an interrupted restore may need to be repeated.
Previous corrupt bytes are retained for manual recovery but must be repaired
before they can pass snapshot validation. Unlisted files are left in place.

### Cached covers and visual suggestions

New downloaded covers are oriented and reduced to a maximum edge of 640 pixels.
Their format and transparency are preserved. Existing small
covers keep their bytes. Content hashes keep cache URLs stable across unchanged
detail refreshes.

    lamp --data data prepare-covers --limit 1000

This command processes cached covers locally, saves a metadata snapshot first
and reports byte counts and failures. Repeat it to process the next batch;
prepared covers are skipped. Resizing replaces the cached image with a smaller
copy, so preserve original images separately if you need them. Metadata snapshots
do not restore original cover pixels. The command makes no network requests.

The preview's **Similar covers (experimental)** compares local 64-bit difference
hashes after a scrape or cover preparation. It returns up to 8 suggestions within
8 bit differences, scans at most 100,000 loaded albums, skips uniform hashes and
supports dismissal. It never merges identities or affects duplicate filtering.
Shared photography can produce false positives; crops and different angles can
be missed. The threshold has synthetic regression coverage, not a measured
accuracy rate on real products.

## Politeness and scope

- Global cap of 8 requests/second by default (tune with --rps), 1
  request/second per seller host, honest User-Agent, backoff on errors, and a
  hard stop per seller if Yupoo answers 429/403 repeatedly.
- Covers and metadata only - detail photos are never downloaded; the viewer
  links back to Yupoo for those.
- HTML responses are limited to 8 MiB after decompression. Larger pages fail
  with a size-limit error.
- Public pages only. No login, no paywall, no captcha handling.
- The viewer makes no request beyond this machine. Agent links are built
  locally from item ids, and the optional QC photo lookup is a plain link
  that reaches rep.tools only when you follow it.

## Development

    cargo fmt --check
    cargo clippy --all-targets -- -D warnings
    cargo test                    # fixture-based, no network
    cargo test -- --ignored       # canary against a saved live sample

The ignored test checks the parsers against a real page saved by hand to
`data/debug/sample-list.html` - fetch a seller's `albums?page=1` there and the
canary says whether the selectors still match live Yupoo markup (the test's
comment has the command). Nothing writes that file for you: a scrape only
dumps pages it *failed* to parse, which is the opposite of a canary. So on a
fresh clone it fails by design rather than passes vacuously.

The tests need node: they run the viewer suite and check the scraper's
duplicate, URL and age rules against the viewer's own code, and they fail
rather than skip when node is missing.

The viewer is dependency-free vanilla JS with no build step - edit
`viewer/*.js` and reload. Its visual system is recorded in `DESIGN.md`
(tokens, named rules, component behaviour) and the product record it serves is
in `PRODUCT.md`; read `DESIGN.md` before changing anything in `viewer/`,
because the card geometry there is a contract `app.js` reads back out of the
stylesheet at runtime. Its logic modules have their own tests, run by
`cargo test` and directly with:

    node --test tests/viewer/*.test.cjs

For strict type checking of the viewer's logic modules, controllers and compile-only
contract tests, run `npm ci` then `npm run typecheck`. TypeScript and the Node
type declarations are development tools only; the check emits no files and
the browser still loads the same JavaScript. `viewer/controller-types.d.ts`
describes catalogue state and the elements owned by `index.html`; a Node test
checks the declared element IDs and tags. `viewer/browser-globals.d.ts` gives the
script loading path the same helper signatures as the Node test imports.
Compiler checks cover saved-value reads, DOM access and controller calls;
browser verification still checks focus, layout and keyboard behaviour.

To work on the viewer without scraping anything, generate synthetic data:

    cargo run --example make_demo_data -- demo-data
    lamp --data demo-data serve

That writes 5 sellers x 2000 albums, with generated gradient covers for 2
albums in every 3, spread across each seller. The rest are left cover-less
on purpose, so the viewer's titled colour-block fallback is on screen too.
Fixtures include known and unknown prices, changed prices and gone listings.
For a larger metadata fixture with fewer generated images:

    cargo run --example make_demo_data -- demo-data-100k 10 10000 100

The final argument limits generated covers per seller. Use synthetic data to
check previews, cross-tab saves, backup/import, Back navigation and keyboard
focus at narrow and desktop widths. To verify packaging:

    cargo build --locked --release

Run the resulting binary from a directory outside the clone and check that the
page, scripts and styles return successfully; `cargo test` checks that every
asset `index.html` loads is built into the binary.

Yupoo serves 2 different list-page templates and the parser handles both.
Every listing page is checked for at least 3 distinct album links the parser
did not read. Such a page fails the seller as a possible layout change before
it can end the listing and mark cached albums gone, and is saved to
`data/debug/<seller>-pageN.html` (dry runs save nothing). An empty first page
without such links reports an empty or closed seller. The threshold can miss
fewer unread links; synthetic tests do not establish real-site coverage.
A listing that ends with 5 or more known albums missing, such as an empty
later page from a soft block, marks none of them gone or resolved and reports
an error. The next run confirms it only if it lists at least as many albums
and is not aborted by blocking; 2 identical truncations in a row still mark
the missing albums gone. A
truncated listing that misses fewer albums still marks them gone until they
are listed again.

Apache-2.0 licensed. If you redistribute this work or build on it, keep the attribution in [NOTICE](NOTICE).

---

<p align="center"><sub>Still looking for an honest seller.<br>Made by <a href="https://github.com/EquityBot">EquityBot</a>. <i>Ad gloriam eius, non meam.</i></sub></p>
