//! Ported from tests/test_workflow_security.py: fail-closed contracts for
//! the security-bearing policy files. A change here must be reviewed for its
//! security effect and the expected text updated explicitly.
mod common;

use sha2::{Digest, Sha256};

const EXPECTED_CI: &str = r#"name: ci
on:
  push:
    branches: [main]
  pull_request:
  workflow_dispatch:

concurrency:
  group: ci-${{ github.ref }}
  cancel-in-progress: true

permissions:
  contents: read

jobs:
  test:
    timeout-minutes: 30
    strategy:
      fail-fast: false
      matrix:
        os: [ubuntu-latest, windows-latest]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
        with:
          persist-credentials: false
      # The Rust tests run the viewer's node suite and the browser parity
      # checks, and fail rather than skip without node, so it is installed
      # explicitly. The separate type check does not execute tests.
      - uses: actions/setup-node@949feb2413d6458794dcd2491c4babbbce0c15c1 # v7.1.0
        with:
          node-version: "22"
          package-manager-cache: false
      - run: npm ci --ignore-scripts
      - run: npm run typecheck
      # rust-toolchain.toml pins the compiler and its clippy and rustfmt components.
      - run: cargo fmt --check
      - run: cargo clippy --locked --all-targets -- -D warnings
      - run: cargo test --locked
      - run: cargo build --locked --release
"#;

const EXPECTED_DEPENDABOT: &str = r#"version: 2
updates:
  - package-ecosystem: npm
    directory: /
    schedule:
      interval: weekly
    cooldown:
      default-days: 7
  - package-ecosystem: github-actions
    directory: /
    schedule:
      interval: weekly
    cooldown:
      default-days: 7
    ignore:
      # The release-policy reusable workflow is pinned by SHA-256 in
      # tests/repo_policy.rs, so a bot bump can only fail CI.
      # Bump it by hand and move the pin in the same commit.
      - dependency-name: ryanduguid/release-policy/.github/workflows/attribution-policy.yml
  - package-ecosystem: cargo
    directory: /
    schedule:
      interval: weekly
    cooldown:
      default-days: 7
    groups:
      rust-dependencies:
        patterns:
          - "*"
"#;

const EXPECTED_SECURITY: &str = r#"# Security policy

## Supported versions

Security fixes are applied to the latest version on the default branch.

## Reporting a vulnerability

Report it privately through GitHub's private vulnerability reporting:
https://github.com/EquityBot/DiogenesLamp/security/advisories/new. Do not open
an issue for a suspected security vulnerability. Include a clear description,
reproduction steps, impact, and any suggested mitigation.

Use fabricated catalogue pages, source configuration and reproduction data only. Do
not submit client or private catalogue data, credentials, access tokens, cookies,
secrets or `.env` content.

## What this tool does and does not do

DiogenesLamp fetches public Yupoo catalogue pages, writes JSON and thumbnails
to a local directory, and serves a local viewer. It does not log in, hold
OAuth tokens, or write back to Yupoo.

Treat scraped titles, HTML and JSON as untrusted data in the viewer. Do not
run the importer with elevated privileges, and do not point it at a less
trusted user's working directory.

The scraper talks to the network. `lamp import` reads a local link dump and
writes `sources.toml` without fetching anything.

Keep `sources.toml` free of credentials. The expected input is a dump of public
http or https links: Yupoo catalogue URLs, which are the only ones scraped, plus
non-Yupoo shop links (taobao, tmall, weidian) and link aggregators, which are
kept as a link-out directory and never fetched.
"#;

const EXPECTED_TOOLCHAIN: &str = r#"[toolchain]
channel = "1.99"
components = ["clippy", "rustfmt"]
profile = "minimal"
"#;

/// Pin the reviewed attribution workflow without duplicating its script.
const ATTRIBUTION_SHA256: &str = "2dd1e990c00ecc3cb8f873095a92a55e588c72d9c9badac6db5e9793e2d95a7a";

fn read(relative: &str) -> String {
    common::read(&common::root().join(relative))
}

fn assert_exact(actual: &str, expected: &str, label: &str) {
    assert!(
        actual == expected,
        "{label} no longer matches its reviewed fail-closed contract; review the security effect and update this test explicitly"
    );
}

#[test]
fn active_workflow_inventory_is_exact() {
    let mut names: Vec<String> = std::fs::read_dir(common::root().join(".github/workflows"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".yml") || n.ends_with(".yaml"))
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            "ci.yml",
            "codeql.yml",
            "no-ai-attribution.yml",
            "scorecard.yml",
            "snyk.yml"
        ]
    );
}

#[test]
fn ci_workflow_is_exact() {
    assert_exact(&read(".github/workflows/ci.yml"), EXPECTED_CI, "ci.yml");
}

#[test]
fn attribution_workflow_is_pinned() {
    let digest = Sha256::digest(read(".github/workflows/no-ai-attribution.yml").as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    assert_exact(&hex, ATTRIBUTION_SHA256, "no-ai-attribution.yml");
}

#[test]
fn dependabot_document_is_exact() {
    assert_exact(
        &read(".github/dependabot.yml"),
        EXPECTED_DEPENDABOT,
        ".github/dependabot.yml",
    );
}

#[test]
fn security_document_is_exact() {
    assert_exact(&read("SECURITY.md"), EXPECTED_SECURITY, "SECURITY.md");
}

#[test]
fn toolchain_is_pinned() {
    assert_exact(
        &read("rust-toolchain.toml"),
        EXPECTED_TOOLCHAIN,
        "rust-toolchain.toml",
    );
}

/// Releases are built from the committed lock file only.
#[test]
fn cargo_lock_is_tracked() {
    let listed = std::process::Command::new("git")
        .args([
            "-C",
            &common::root().to_string_lossy(),
            "ls-files",
            "--error-unmatch",
            "Cargo.lock",
        ])
        .output()
        .unwrap();
    assert!(listed.status.success(), "Cargo.lock must be committed");
}
