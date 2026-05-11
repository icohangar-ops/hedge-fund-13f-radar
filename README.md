# Hedge Fund 13F Radar

Hedge Fund 13F Radar turns the Dave Wang 13F automation prompt into a repeatable open-source workflow for tracking hedge fund conviction, initiations, exits, and sector rotation.

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

