# Design system: DiogenesLamp

## Overview

**Creative North Star: 'The Stock Book'**

A stock book is the leaf a collection keeps for its specimens: a page ruled
before anything is mounted on it, windows cut at a fixed pitch, a folded paper
corner holding each piece, and a printed line of catalogue figures underneath.
Nothing on that page competes with what is mounted on it, which is the whole
reason the form survived: the page is apparatus, the specimens are the subject.

DiogenesLamp is tens of thousands of cover photographs judged at a glance, so
it inherits that discipline literally. The chrome is 2 inks on one ground.
Every colour a visitor sees comes from the covers themselves. There are no
rounded corners, no shadows on the leaf, and no card containers: the rules are
printed once across the page as a background, and the windows are cut into it.

The visual anti-reference is the thing this replaced and the thing this
category always ships: a grid of floating rounded cards with elevation
shadows, a single warm accent, and a sticky pill-shaped filter bar.

**Key Characteristics:**

- Two inks and one ground. A third colour is a defect.
- The lattice is printed once, never per card.
- Every state reads with the colour taken away.
- Figures are tabular and hold their width while the catalogue streams in.
- More windows per screen is always the better answer.

## Colours

Neutrals plus one accent, because the visitor came to operate. The ground is
dark by default from the use scene, not the category: this is read at a desk
at night with the room lights down and the covers the only lit thing on the
screen. The light leaf is a cool printed stock, deliberately not a warm cream.

### Primary

- **Second Ink** (`#ff6a4d` dark / `#b8301a` light): the correction ink of a
  printed catalogue. It carries arrivals, the matched span of a query, the
  mounted corner, the checked mark, the load rule, the caret and the selection.
  Nothing decorative ever wears it.

### Neutral

- **Leaf** (`#0e1013` dark / `#e7e9ec` light): the page. Body ground and the
  window ground behind a cover that failed to load.
- **Leaf Raised** (`#14171b` / `#f3f5f7`): the head band and the shops panel,
  the 2 surfaces that sit above the page rather than on it.
- **Rule** (`#262c33` / `#c8ced5`): the printed lattice, the hairline under a
  window, the box around a purchase mark.
- **Rule Strong** (`#3a434d` / `#9aa4ae`): the baseline under a form field,
  the border of a control, the scrollbar thumb.
- **Ink** (`#e9ebee` / `#14171b`): titles, control text, the focus ring.
- **Ink Faint** (`#9aa3ad` / `#545c66`): sellers, tallies, purchase marks,
  placeholders. Both values clear 4.5:1 on their own leaf.

### Named rules

**The Two Ink Rule.** The chrome uses the leaf, the ink and one second ink.
Every other colour on the screen is a photograph. A platform that wants its
own brand colour, a status that wants amber, a category that wants a hue: all
of them are refused, and the state finds a shape instead.

**The Cover Sovereignty Rule.** Only functional marks belong over a cover.
Current cards use the arrival tag, price figure and corner mount. Adding a
fourth requires removing one. The retained duplicate-count mark does not
appear after newest-listing selection.

## Typography

**Body Font:** the platform UI stack (`system-ui`, `-apple-system`,
`Segoe UI`, sans-serif)
**Label/Mono Font:** the platform mono stack (`ui-monospace`, `Cascadia Mono`,
`Segoe UI Mono`, `Menlo`, monospace)

**Character:** a working catalogue, not a printed one. The system stack is a
constraint recorded honestly rather than a choice: the viewer ships with no
build step, no bundler and no external requests, so a self-hosted face would
be a binary in a repository whose whole claim is that it has no dependencies.
The printed register is carried by the mono figures, the tracked uppercase
labels and the rules, not by a text face.

There is no display voice on this page, by design. A leaf of specimens has no
headline.

### Hierarchy

- **Note** (400, 22px, 1.5): the empty and loading states, the only place the
  page speaks a sentence.
- **Body / Title** (400, 14px, 1.32 to 1.45): captions, controls, panel links.
  Card titles clamp to 2 lines, one on narrow.
- **Label** (400, 11px, mono, tabular): sellers, tallies, purchase marks,
  duplicate counts, dates, panel headings.
- **Brand** (700, 11px, uppercase, 0.14em): the name in the head band.

### Named rules

**The Tabular Figure Rule.** Every number the interface owns is set in the
mono stack with `font-variant-numeric: tabular-nums`, and any figure that
changes while the catalogue streams in is held at the width of its largest
value. A count that widens as it grows moves the controls beside it.

**The Eleven Pixel Floor.** No functional text below 11px, including the marks
that look decorative. The `sr-only` class is the only exemption.

## Layout

A virtualised absolute-positioned lattice. Card geometry lives in
`:root` as `--card-w`, `--card-h`, `--cover-h` and `--gap`, and `app.js` reads
those values back out of the computed style rather than duplicating them, so a
stylesheet change moves the real layout. Anything that changes the pitch must
leave it readable from `:root`, and the geometry is re-read on resize and
whenever the strip page opens.

Browsing uses the lattice at 220 by 288 with a 214 cover and a 10px gutter,
or the narrow lattice at 158 by 202 with a 152 cover and an 8px gutter below
720px. The retained strip geometry is 300 by 384 with a 300 cover and a 14px
gutter, or 300 by 300 with a 220 cover and an 8px gutter on narrow.

The index and viewer select the newest duplicate listing before display
filters, following the rules in [PRODUCT.md](PRODUCT.md) and
[docs/reference.md](docs/reference.md#serve-and-browse). This also applies
when loading older catalogues.
The **Hide duplicates** checkbox and strip code remain in the viewer, but
they cannot restore older copies or offer a duplicate-count button for them.

The lattice itself is 2 `repeating-linear-gradient` layers on the spacer,
at the card pitch, so the ruling costs one paint rather than a border on each
of the hundred-odd nodes in the window. The spacer is exactly as wide as the
columns it holds and is centred, so the page never rules ground no window can
sit on.

The head band is sticky above 720px and static below it, where a wrapping
toolbar would otherwise hold a third of the viewport permanently.

## Elevation and depth

Flat, with one exception. The leaf has no shadows at all: depth is the ruling
and the 2 raised surface tones. The shops panel is the one element that
physically sits above the page, and it is the one element that casts a shadow.

### Shadow vocabulary

- **Lift** (`0 2px 4px rgb(0 0 0 / 0.45), 0 12px 28px rgb(0 0 0 / 0.38)` dark;
  `0 2px 4px rgb(16 20 30 / 0.10), 0 12px 28px rgb(16 20 30 / 0.12)` light):
  the slide-over panel only.

### Named rules

**The Printed Page Rule.** A surface on the leaf is distinguished by a rule or
by a tone, never by a shadow. A shadow on the page means the element is
physically above it, and only the panel is.

## Shapes

Radius is zero everywhere, without exception. Corners are cut, not rounded:
that single decision is what separates this page from the card grid it
replaced, and softening one corner starts the drift back.

Marks are drawn SVG from one sheet of `<symbol>` definitions in `index.html`,
referenced with `<use>` and stroked at 1.8 from `currentColor`. Symbols carry
geometry only, never `fill` or `stroke` attributes, because a presentation
attribute on the path beats the inherited value and the shadow tree cannot be
reached any other way. No glyph or emoji ever stands in for a mark.

The corner mount is the form's signature: a paper wedge folded over the
window's top right corner, outlined in the faint ink when the album is not
kept and filled solid in the second ink when it is.

## Components

- **Window.** No border, no radius, no background of its own beyond the leaf.
  Hover outlines it in the faint ink, focus outlines it in the ink at a 1px
  offset so the ring lands in the gutter rather than over the cover. It never
  moves or grows: a transform here fights the transform that places it.
- **Cover.** Fills its window edge to edge with a 1px rule beneath. Behind it
  sits a titled colour block on a stable per-album hue, shown when Yupoo has
  no cover at all. The hue is set through the CSSOM, never as a style
  attribute: the page is served under a policy with no `unsafe-inline`, so a
  style attribute in parsed markup is dropped.
- **Caption.** A clamped title, then a mono line of seller and purchase marks.
  The marks keep their width and the seller name gives way, because a
  truncated seller is readable and a mark clipped mid-word is not. On narrow
  the seller stands down entirely.
- **Preview.** The title is a button within the existing caption geometry.
  It opens a native modal dialog with the cached cover and full metadata. The
  dialog uses the existing ink and leaf tokens, square rules and no extra
  shadow. It becomes a single scrollable column on narrow screens. Escape
  returns focus to the originating card; edited forms warn before discarding.
- **Saved work.** Additional filters and backup import use a native disclosure
  beneath the main filters. Notes and collections live in the preview, leaving
  the cover marks and virtual grid dimensions unchanged. A second disclosure
  of the same form, **Links and prices**, holds the agent, lookup and price
  rate preferences; a converted price prints in the existing price figure's
  footprint, marked ≈ with its currency code, never as a new mark on the cover.
- **Form field.** A baseline rule rather than a box, thickening into the
  second ink on focus. Selects carry a drawn caret as a data URI rather than
  the platform arrow.
- **Mark box.** The checkbox toggles are a ruled square with a drawn tick;
  the native input stays in the accessibility tree and keeps the keyboard, and
  only the paint is ours. The tick is hidden by giving up its stroke, never
  its opacity, which would take the box with it.
- **Arrival.** A 2px inset rule in the second ink across the top of the window
  plus a `NEW` tag, 2 carriers so the state survives without colour.
- **Price.** A catalogue figure printed at the foot of the window, on the leaf
  rather than floating on the cover. It is a figure,
  not a badge: mono, tabular, no colour of its own.
- **Export.** A drawn mark and a plain label. Every button in the head band is
  a ruled rectangle; the second ink is reserved for the 2 that undo
  something, and export is not one of them.
- **Strip page.** The retained duplicate-group view uses a wider pitch with
  seller names in the main ink and posted dates. It has no normal card entry
  point after newest-listing selection.

## Do's and don'ts

**Do** put a new state on a shape, a rule or a mark before reaching for
colour, and confirm it still reads in greyscale.

**Do** read geometry from `:root` and let `app.js` measure it. Hard-coding a
pitch anywhere else desynchronises the virtualised placement and overlaps
rows.

**Do** hold any figure that changes during a load at its maximum width.

**Don't** add a radius, a shadow on the leaf, a gradient, or a third colour.

**Don't** paint anything new over a cover without removing something else.

**Don't** use a glyph, an emoji or an icon font where a mark belongs. Add a
`<symbol>` to the sheet in `index.html` and stroke it from CSS.

**Don't** add per-card borders, per-card transitions or per-card entrance
animation. The lattice is one paint and the grid holds forty thousand items.
