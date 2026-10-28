# Changelog

## 0.1.0 (Rust port, unreleased)

`lamp` is now a single Rust binary. It replaces the Python package
(`src/lamp`), its wheel packaging and its pytest suite. The command,
subcommands, flags, exit codes, status lines and written files match the
Python implementation at d8c03c3 on the fabricated samples and generated
differential corpus, apart from the divergences below. The viewer is
unchanged.

### Documented divergences

- **Re-encoded covers.** A cover that is resized or rotated (downloads and
  `prepare-covers`) is re-encoded by the `image` crate, not Pillow, so its
  bytes, `after_bytes` and `thumb_version` differ. WebP covers are re-encoded
  losslessly (Pillow wrote lossy quality 85), so they are larger. Covers that
  need no change keep their bytes, as before.
- **JPEG fingerprints.** `cover_hash` uses a port of Pillow's greyscale
  conversion and LANCZOS resampling, and matches exactly for PNG and WebP. JPEG
  covers decode through zune-jpeg rather than libjpeg-turbo: in a probe of 75
  generated JPEGs, 12 fingerprints differed, by 1 to 3 of 64 bits (most by 1),
  well inside the viewer's similarity threshold.
- **Image format coverage.** Fingerprints and resizing read JPEG, PNG and WebP.
  A GIF or other format placed in the cover cache by hand gets an empty
  `cover_hash` (Pillow fingerprinted it).
- **Snapshot archives.** Member names and contents match; the archive bytes do
  not (a different deflate implementation and member metadata).
- **Viewer assets** are built into the binary. Their responses carry no
  `Last-Modified` header and never answer `If-Modified-Since` with 304; data
  files under `/data/` behave as before. The `Server` header names
  DiogenesLamp instead of the Python version. The "viewer files not found"
  error can no longer occur.
- **Uncaught Python exceptions.** Where the Python CLI crashed with a traceback
  (an unreadable or invalid `sources.toml`, an unsafe source id on import, a
  port already in use, an unreadable dump file), `lamp` prints `error: ...`
  and exits 1.
- **Short listings.** A listing that ends with 5 or more known albums
  missing (previously listed, or failed and never recorded; an empty or
  repeated page part-way through, as a soft block could cause) marks none of
  them gone and keeps their pending failures. The seller reports an error and
  records `short_listing`, the number listed, in its run summary. The next run
  confirms the shortfall, marking the missing albums gone, only if it lists at
  least that many albums and is not aborted by blocking. The
  Python version marked them gone at once.
- **Unexpected seller failures.** A seller that fails unexpectedly reports
  `unexpected error: <message>` and writes the message to stderr instead of a
  Python traceback and exception repr.
- **Network error wording.** Messages that quote the HTTP library's own text
  (`request failed for ...: <detail>`, DNS failures on Linux) carry ureq's or
  the operating system's wording, not requests' or getaddrinfo's. Charsets
  other than UTF-8, ASCII and Latin-1 decode as UTF-8, and a page with no
  declared charset decodes as UTF-8 rather than by charset detection. A
  non-ASCII host is encoded with UTS 46 rather than Python's IDNA 2003 codec.
  A stalled response body times out after 120 seconds in total rather than 30
  seconds per read.
- **JSON edge cases.** A JSON string holding a lone surrogate escape
  (`"\ud800"`) is refused as corrupt metadata, because a Rust string cannot hold
  it; Python accepted it on read but could not write it back. Nesting deeper
  than 4000 levels is refused with an error instead of a RecursionError.
- **TOML values.** A datetime value in `sources.toml` name, URL or platform is
  kept as its TOML text.
- **Terminal output** is never coloured (Python 3.15's argparse colours help
  on a terminal) and is always UTF-8. On Windows, as with Python, line breaks
  are written as CRLF.
- **Unicode tables.** Character classes follow Python 3.15 (Unicode 17); NFKC
  normalisation and IDNA mapping come from Rust crates, which can differ for
  characters added in the newest Unicode versions.

### Development

- `cargo test` replaces pytest. It runs the viewer's node suite and the
  browser parity checks, and fails rather than skips without node.
- `cargo run --example make_demo_data` replaces `tests/make_demo_data.py`
  (same shape of data; its pseudo-random titles and ages differ).
- The `fixture-transport` feature (differential testing only) answers HTTPS
  requests from a local fixture directory.
