# Repository instructions

Before changing code, read the README's Development and Politeness and scope
sections. The editable installation is required because the viewer is served
from the source tree.

- For `viewer/` changes, read [DESIGN.md](DESIGN.md) and [PRODUCT.md](PRODUCT.md).
  Preserve the CSS geometry used by JavaScript and the keyboard interaction rules.
- Keep verification fixture-based. The README documents synthetic demo data and
  separate manual parser canaries; a missing saved sample is not a passing canary.
- Keep private source lists, fetched catalogues and debug captures outside tracked
  files. Preserve request limits, backoff and public-page-only access.

Before handoff, run the affected checks from [ci.yml](.github/workflows/ci.yml)
and the README. Check that Node-backed viewer tests ran rather than skipped.
Report manual canaries and live scraping separately; do not start a scrape merely
to verify a documentation or viewer change.
