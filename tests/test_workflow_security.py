"""Fail-closed contracts for current security-bearing policy files."""

from hashlib import sha256
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
WORKFLOW_DIR = ROOT / ".github" / "workflows"

EXPECTED_ACTIONS = {
    "actions/checkout": (
        "3d3c42e5aac5ba805825da76410c181273ba90b1",
        "v7.0.1",
        "v7.0.1",
    ),
    "actions/setup-python": (
        "5fda3b95a4ea91299a34e894583c3862153e4b97",
        "v7.0.0",
        "v7.0.0",
    ),
    "actions/setup-node": (
        "820762786026740c76f36085b0efc47a31fe5020",
        "v7.0.0",
        "v7.0.0",
    ),
    "astral-sh/setup-uv": (
        "20cfd1bf945f4377ade1205e4dbc17946fc9a30d",
        "v10.0.1",
        "v10.0.1",
    ),
}


def _uses(action: str, prefix: str) -> str:
    commit, comment, _resolved_release = EXPECTED_ACTIONS[action]
    return f"{prefix}{action}@{commit} # {comment}"


EXPECTED_CI = "\n".join(
    [
        "name: ci",
        "on:",
        "  push:",
        "    branches: [main]",
        "  pull_request:",
        "  workflow_dispatch:",
        "",
        "concurrency:",
        "  group: ci-${{ github.ref }}",
        "  cancel-in-progress: true",
        "",
        "permissions:",
        "  contents: read",
        "",
        "jobs:",
        "  test:",
        "    timeout-minutes: 30",
        "    runs-on: ubuntu-latest",
        "    strategy:",
        "      fail-fast: false",
        "      matrix:",
        '        # Oldest and newest supported, so the README\'s "Python 3.11+" claim',
        "        # is actually tested rather than assumed.",
        '        python-version: ["3.11", "3.13"]',
        "    steps:",
        _uses("actions/checkout", "      - uses: "),
        "        with:",
        "          persist-credentials: false",
        _uses("actions/setup-python", "      - uses: "),
        "        with:",
        "          python-version: ${{ matrix.python-version }}",
        "      # The viewer's logic modules have node tests. pytest runs them",
        "      # through a wrapper that skips when node is missing, so installing it",
        "      # explicitly is what keeps that suite from silently disappearing on a",
        "      # runner image change - and is why there is no separate node step:",
        "      # it would run the same tests a second time.",
        _uses("actions/setup-node", "      - uses: "),
        "        with:",
        '          node-version: "22"',
        _uses("astral-sh/setup-uv", "      - uses: "),
        "        with:",
        '          version: "0.12.0"',
        "          enable-cache: true",
        "      - run: uv run --locked --extra dev ruff check .",
        "      - run: uv run --locked --extra dev mypy src/lamp",
        "      - run: uv run --locked --extra dev pytest",
        "",
    ]
)

EXPECTED_WORKFLOWS = {
    "ci.yml": EXPECTED_CI,
}

# Pin the reviewed script too, without duplicating its embedded Python here.
EXPECTED_WORKFLOW_HASHES = {
    "no-ai-attribution.yml": "c694fb285f08e99a13f472ab99861b699038e659712a844a718403fddbd63daf",
}
EXPECTED_WORKFLOW_NAMES = sorted(EXPECTED_WORKFLOWS.keys() | EXPECTED_WORKFLOW_HASHES.keys())

EXPECTED_DEPENDABOT = """\
version: 2
updates:
  - package-ecosystem: github-actions
    directory: /
    schedule:
      interval: weekly
    cooldown:
      default-days: 7
  - package-ecosystem: uv
    directory: /
    schedule:
      interval: weekly
    cooldown:
      default-days: 7
    groups:
      python-dependencies:
        patterns:
          - "*"
"""

EXPECTED_SECURITY = """\
# Security policy

## Supported versions

Security fixes are applied to the latest version on the default branch.

## Reporting a vulnerability

Please use this repository's private vulnerability reporting feature. Do not
open a public issue for a suspected security vulnerability. Include a clear
description, reproduction steps, impact, and any suggested mitigation.

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

The importer talks to the network. Keep `sources.toml` free of credentials;
Yupoo catalogue URLs are the only expected input.
"""


def _assert_exact(actual: str, expected: str, label: str) -> None:
    assert actual == expected, (
        f"{label} no longer matches its reviewed fail-closed contract; "
        "review the security effect and update this test explicitly"
    )


def _workflow_documents() -> dict[str, str]:
    paths = sorted((*WORKFLOW_DIR.glob("*.yml"), *WORKFLOW_DIR.glob("*.yaml")))
    return {path.name: path.read_text(encoding="utf-8") for path in paths}


def test_active_workflow_inventory_is_exact():
    assert sorted(_workflow_documents()) == EXPECTED_WORKFLOW_NAMES


@pytest.mark.parametrize("filename", ["ci.yml"])
def test_active_workflow_document_is_exact(filename: str):
    _assert_exact(_workflow_documents()[filename], EXPECTED_WORKFLOWS[filename], filename)


@pytest.mark.parametrize("filename", EXPECTED_WORKFLOW_HASHES)
def test_script_workflow_document_is_exact(filename: str):
    actual = _workflow_documents()[filename]
    _assert_exact(
        sha256(actual.encode("utf-8")).hexdigest(),
        EXPECTED_WORKFLOW_HASHES[filename],
        filename,
    )


def test_dependabot_document_is_exact():
    actual = (ROOT / ".github" / "dependabot.yml").read_text(encoding="utf-8")
    _assert_exact(actual, EXPECTED_DEPENDABOT, ".github/dependabot.yml")


def test_security_document_is_exact():
    actual = (ROOT / "SECURITY.md").read_text(encoding="utf-8")
    _assert_exact(actual, EXPECTED_SECURITY, "SECURITY.md")

