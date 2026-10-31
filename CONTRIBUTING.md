# Contributing

Report bugs and request features in
[GitHub issues](https://github.com/EquityBot/DiogenesLamp/issues). For a
suspected security vulnerability, follow [SECURITY.md](SECURITY.md) instead of
opening an issue.

## Pull requests

Propose changes as a pull request against `main`. A change lands on `main` only
when the `test (ubuntu-latest)` and `test (windows-latest)` checks from
[ci.yml](.github/workflows/ci.yml) pass. The `No AI attribution` check fails a
pull request whose title, commit trailers or identities credit an AI tool.

Run what CI runs before opening one. The tests need node; they run the viewer
suite and fail rather than skip without it:

```bash
npm ci --ignore-scripts
npm run typecheck
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --locked --release
```

New functionality and bug fixes come with automated tests in `tests/`, run by
`cargo test --locked` in CI. Keep verification fixture-based: use fabricated
catalogue pages and the synthetic demo data the README's Development section
describes, never private source lists, fetched catalogues, credentials or
cookies. Read [DESIGN.md](DESIGN.md) and [PRODUCT.md](PRODUCT.md) before
changing `viewer/`.

`tests/repo_policy.rs` pins the CI workflow, the Dependabot configuration,
SECURITY.md and the toolchain file. A change to one of them is a security
change: update its expected text in the same pull request and say why it
changed.
