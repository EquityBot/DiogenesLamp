# Reference

The [README](../README.md#use) gives the 3 steps: import, scrape and browse.
This page holds the flags and viewer behaviour behind them.

## Import and `sources.toml`

To leave a seller out of scrapes - a catalogue too large to be worth the
hours, say - add `skip = true` to its entry. Commenting it out would not
last, since `lamp import` rewrites the file from your dump.

## Scrape

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

## Serve and browse

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
