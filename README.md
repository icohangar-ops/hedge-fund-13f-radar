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
- high-conviction ideas by fund
- CHP-style verification status

## Required Columns

`manager,quarter,ticker,company,sector,shares,prior_shares,value_usd,source_url`

The verifier returns `REQUIRES_HUMAN_VERIFICATION` if source URLs are missing, numeric fields are invalid, or the file does not contain enough managers for cross-fund analysis.

This is not investment advice. It is a research workflow and data-quality gate.

## Demo

📺 [Watch the demo](demos/$(basename "$video")) — slide-style walkthrough of key features and usage.

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

