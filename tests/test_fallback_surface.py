"""Evidence for evidence/matrix.yaml C013: the pure-Python fallback surface.

README (#python-bindings-pyo3): "The pure-Python fallback surface (analyze_13f)
still works without the native module." This test runs analyze_13f in a
subprocess where importing hedge_fund_13f_radar._native is blocked outright,
proving the fallback workflow never depends on the native extension.
"""

import os
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]

BLOCKED_NATIVE_DRIVER = """\\
import sys

class _NoNative:
    def find_spec(self, fullname, path=None, target=None):
        if fullname == "hedge_fund_13f_radar._native":
            raise ImportError("native engine blocked (fallback-surface test)")
        return None

sys.meta_path.insert(0, _NoNative())

import hedge_fund_13f_radar

assert hedge_fund_13f_radar.HAS_NATIVE_ENGINE is False, "native module was importable"

from hedge_fund_13f_radar.core import analyze_13f

report = analyze_13f("examples/holdings_13f.csv")
print("FALLBACK_STATUS:", report.verification.status)
print("FALLBACK_CONSENSUS:", ",".join(report.consensus_tickers))
"""


def test_analyze_13f_runs_without_native_module():
    proc = subprocess.run(
        [sys.executable, "-c", BLOCKED_NATIVE_DRIVER],
        cwd=str(REPO_ROOT),
        env=dict(os.environ, PYTHONPATH=str(REPO_ROOT / "src")),
        capture_output=True,
        text=True,
        timeout=120,
    )
    assert proc.returncode == 0, proc.stderr
    assert "FALLBACK_STATUS: CLEAR" in proc.stdout
    assert "GOOG" in proc.stdout
