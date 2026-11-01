# Synthetic browser qualification

These opt-in checks use the existing Rust demo generator, release binaries and
Node's standard library to drive an installed Chromium-compatible browser over
CDP. They do not download a browser or fetch a real catalogue.

Prerequisites: a POSIX host, Rust, Node 22 or newer and an installed Chromium-compatible
browser. Windows requires an isolated job runner for reliable process-tree cleanup
and is rejected by this driver. Linux qualification is recorded separately from
other POSIX hosts. Set `BROWSER_BIN` to its executable path, then run from the repository:

```sh
npm run test:browser
npm run bench:browser -- --output /absolute/path/results.json
```

Both commands build the release binaries and generate a disposable fixture with
100,000 raw records and 10 sellers. Deduplication and default filters reduce the
selected and visible counts; the JSON records those counts separately.
`test:browser` runs once at 1440 × 900 and 390 × 844. `bench:browser` runs 5
repetitions at each viewport and also checks exact import/export of 1,000 and
10,000 fabricated saved notes. These are desktop viewport emulations, not mobile
device measurements. Both check keyboard entry, virtualised navigation, preview
focus and saved-field round trips across fresh browser contexts.

Each sample records a fresh browser context followed by a warm reload. Neither
is an OS-cache-cold measurement. First-card and completion timings are observed
by the page and the polling driver; completion includes polling delay. The
nearest-rank p95 of 5 samples is their maximum, not a stable population estimate.
Raw samples accompany every summary. Renderer JS heap is a snapshot. On Linux,
the driver also samples aggregate RSS for its child process groups every 250 ms;
shared pages are counted per process and short peaks may be missed. That figure
is neither physical memory consumption nor a browser-only peak. Other platforms
report no aggregate RSS measurement.

For already-built or externally staged binaries:

```sh
node tests/browser/catalogue.mjs --lamp /path/lamp --generator /path/make_demo_data --browser /path/chromium --benchmark --runs 5 --output /path/results.json
```

`--runs` accepts 1 to 10. `CARGO_TARGET_DIR` is respected. The driver uses a fresh
temporary profile, downloads and fixture directory, bounded child logs, a
3-minute smoke deadline or 10-minute benchmark deadline and owned-process
cleanup. Browser sandboxing remains enabled. If a host cannot start a sandboxed
browser, run in an appropriate isolated environment; the check does not silently
disable the sandbox. Browser background features are disabled and page requests
outside the local catalogue origin are rejected. This page interception is not
an OS network sandbox. For untrusted builds, add process/network/filesystem
isolation outside the driver.

Results go to stdout and, with `--output`, the supplied file. Failure exits
nonzero. No live user profile, clipboard or existing catalogue data is used.
