"""Run the viewer JS unit tests under node, when node is available."""

import shutil
import subprocess
from pathlib import Path

import pytest

TEST_DIR = Path(__file__).parent / "viewer"


@pytest.mark.skipif(shutil.which("node") is None, reason="node not installed")
def test_viewer_js():
    files = sorted(str(p) for p in TEST_DIR.glob("*.test.cjs"))
    assert files, "no viewer JS test files found - glob broken?"
    result = subprocess.run(
        ["node", "--test", *files],
        capture_output=True,
        text=True,
        timeout=60,
        check=False,  # the assert below reports the failure with full output
    )
    assert result.returncode == 0, result.stdout + result.stderr
