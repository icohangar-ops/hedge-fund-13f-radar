"""Evidence for evidence/matrix.yaml C002: the README Quick Start command.

Runs the exact command documented in README.md (#quick-start) and asserts it
exits 0 and prints the radar report as JSON. Executed by the repo's normal
CI test job (post-install); statically checked by the evidence-matrix gate.
"""

import json
import os
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]


def test_quick_start_analyze_json():
    env = dict(os.environ, PYTHONPATH=str(REPO_ROOT / "src"))
    proc = subprocess.run(
        [
            sys.executable, "-m", "hedge_fund_13f_radar.cli", "analyze",
            "--holdings", "examples/holdings_13f.csv",
            "--json",
        ],
        cwd=str(REPO_ROOT),
        env=env,
        capture_output=True,
        text=True,
        timeout=120,
    )
    assert proc.returncode == 0, proc.stderr
    payload = json.loads(proc.stdout)
    assert payload["quarter"] == "2026Q1"
    assert "GOOG" in payload["consensus_tickers"]
    assert payload["verification"]["status"] == "CLEAR"
