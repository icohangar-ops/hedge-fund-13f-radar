# Hedge Fund 13F Radar

Hedge Fund 13F Radar is a repeatable open-source workflow for tracking hedge fund conviction, initiations, exits, and sector rotation from normalized 13F data.

## Quick Start

```bash
PYTHONPATH=src python3 -m hedge_fund_13f_radar.cli analyze \
  --holdings examples/holdings_13f.csv \
  --json
```

## What It Produces

- manager-level top positions
- initiation, exit, increase, and decrease detection
- cross-fund consensus tickers
- sector rotation table
- high-conviction positions by manager and cross-fund conviction signals
- CHP-style verification status

## Required Columns

`manager,quarter,ticker,company,sector,shares,prior_shares,value_usd,source_url`

The verifier returns `REQUIRES_HUMAN_VERIFICATION` if source URLs are missing, numeric fields are invalid, or the file does not contain enough managers for cross-fund analysis.

This is not investment advice. It is a research workflow and data-quality gate.

## Demo

[Watch the demo](demos/hedge-fund-13f-radar_demo.mp4) — slide-style walkthrough of key features and usage.

---

## Python Bindings (pyo3)

The Rust engine is also exposed as a native Python module via [pyo3](https://github.com/PyO3/pyo3) and [maturin](https://github.com/PyO3/maturin) — same engine, same outputs, driven from Python.

### Build

```bash
python -m venv .venv && source .venv/bin/activate
pip install maturin
maturin develop
```

`maturin develop` compiles the crate with the `python` feature (opt-in in `Cargo.toml`, so plain `cargo build`/`cargo test` are unaffected) and installs `hedge_fund_13f_radar` into the active virtualenv. For a distributable wheel: `maturin build --release`.

### Use

```python
from hedge_fund_13f_radar import Pipeline, FilingBuilder, diff_filings, QuarterlyAggregator

pipeline = Pipeline()
result = pipeline.run(prior_quarter_filings, current_quarter_filings, "2025-Q4", "2026-Q1")

print(result.summary_report())        # full radar report
for diff in result.diffs:             # per-manager position changes
    print(diff["manager_name"], diff["summary"])
for signal in result.consensus_signals:
    print(signal["ticker"], signal["net_direction"])
```

Filing and holding inputs are plain dicts (serde-mapped to the engine's `Filing13F`/`RawHolding` structures); a complete end-to-end walk-through — sample CSV in, engine analysis out — lives in [`examples/python_demo.py`](examples/python_demo.py):

```bash
python examples/python_demo.py
```

Python integration tests mirroring the core Rust test paths are in `tests/test_native_engine.py` (`pytest tests/` — they run in CI against the built bindings). The pure-Python fallback surface (`analyze_13f`) still works without the native module.

---

## CHP Governance

This repository is hardened with the [Consensus Hardening Protocol (CHP)](https://codeberg.org/cubiczan/consensus-hardening-protocol), Cubiczan's decision-governance layer for multi-agent AI systems.

### Protocol Layers
- **R0 Gate**: All decisions must pass Solvable, Scoped, Valid, Worth_it checks
- **Foundation Disclosure**: 1-3 weakest assumptions, 1-2 invalidation conditions, 1 key vulnerability
- **Adversarial Layer**: Mandatory devil's advocate at Phase 0 and Round 3
- **State Machine**: EXPLORING → PROVISIONAL → PROVISIONAL_LOCK → LOCKED
- **Third-Party Validation**: Independent CONFIRM/REJECT before lock

### Domain Configuration
- **Category**: Finance (CFO Accuracy)
- **Foundation Threshold**: 100
- **CFO Accuracy Guard**: Enabled

### Compliance Artifacts
| File | Purpose |
|------|---------|
| `.chp/STATE_MACHINE.md` | Decision state transitions |
| `.chp/R0_CONFIG.yaml` | Domain-calibrated thresholds |
| `.chp/ADVERSARIAL_PROMPTS.md` | Standardized challenge templates |
| `.chp/CHP_COMPLIANCE.md` | Compliance tracking & audit trail |

### CHP Version
cognitive-mesh-orchestrator 0.1.0 | [Protocol Docs](https://codeberg.org/cubiczan/consensus-hardening-protocol)

## Evidence Matrix

Every capability claim in this file is backed by [`evidence/matrix.yaml`](evidence/matrix.yaml): each row binds a claim to deterministic evidence — a named test, an executable check, a pinned manifest field, or a hashed artifact. The fail-closed verifier (`tools/verify_evidence_matrix.py`, vendored byte-identical from the canonical kit [icohangar-ops/consensus-hardening-protocol](https://github.com/icohangar-ops/consensus-hardening-protocol)) runs in CI on every pull request and every push to `main`, before any install step; a red `evidence-matrix` job means a claim in this file is not evidence-backed.

