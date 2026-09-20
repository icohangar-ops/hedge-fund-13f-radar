#!/usr/bin/env python3
"""Drive the Rust 13F engine from Python.

Build the native bindings once:

    pip install maturin
    maturin develop

Then run this demo:

    python examples/python_demo.py

It reads the same sample data as the pure-Python workflow
(`examples/holdings_13f.csv`), converts each manager's rows into engine
`Filing13F` dicts (prior quarter reconstructed from `prior_shares` via a
linear value approximation), and runs the full Rust pipeline:
diff -> consensus -> sector rotation -> whale tracking -> clustering.
"""

import csv
from datetime import date
from pathlib import Path

import hedge_fund_13f_radar as radar

# CSV sector labels -> engine sector names (serde variant names).
SECTOR_NAMES = {
    "Technology": "Technology",
    "Healthcare": "Healthcare",
    "Finance": "Finance",
    "Consumer Discretionary": "ConsumerDiscretionary",
    "Consumer Staples": "ConsumerStaples",
    "Energy": "Energy",
    "Industrials": "Industrials",
    "Materials": "Materials",
    "Real Estate": "RealEstate",
    "Utilities": "Utilities",
    "Communication Services": "CommunicationServices",
}


def prior_quarter(label):
    """'2026-Q1' -> '2025-Q4'"""
    year, quarter = label.split("-Q")
    if quarter == "1":
        return f"{int(year) - 1}-Q4"
    return f"{year}-Q{int(quarter) - 1}"


def quarter_end(label):
    """'2026-Q1' -> date(2026, 3, 31)"""
    year, quarter = label.split("-Q")
    month, day = {1: (3, 31), 2: (6, 30), 3: (9, 30), 4: (12, 31)}[int(quarter)]
    return date(int(year), month, day)


def load_filings(csv_path):
    """Convert the pure-Python CSV columns into engine filing dicts."""
    with open(csv_path, newline="") as handle:
        rows = list(csv.DictReader(handle))

    managers = {}
    for row in rows:
        label = f"{row['quarter'][:4]}-Q{row['quarter'][-1]}"  # "2026Q1" -> "2026-Q1"
        manager = managers.setdefault(
            row["manager"], {"quarter": label, "prior": [], "current": []}
        )

        shares = int(row["shares"])
        prior_shares = int(row["prior_shares"])
        value = float(row["value_usd"])
        # The CSV records only the current value; approximate the prior value
        # linearly from the share counts.
        prior_value = value * prior_shares / shares if shares else 0.0

        base = {
            "cusip": f"DEMO{row['ticker']}",  # no CUSIP in this CSV — stable stand-in
            "ticker": row["ticker"],
            "name": row["company"],
            "sector": SECTOR_NAMES[row["sector"]],
            "share_type": "SH",
            "discretion": "SO",
            "is_option": False,
        }
        if prior_shares > 0:
            manager["prior"].append({**base, "shares": prior_shares, "value": prior_value})
        manager["current"].append({**base, "shares": shares, "value": value})

    filings = {"prior": [], "current": []}
    for index, (name, data) in enumerate(sorted(managers.items())):
        for bucket in ("prior", "current"):
            label = prior_quarter(data["quarter"]) if bucket == "prior" else data["quarter"]
            holdings = data[bucket]
            aum = sum(h["value"] for h in holdings)
            for h in holdings:
                h["portfolio_weight"] = h["value"] / aum if aum else 0.0
            filings[bucket].append({
                "accession_number": f"DEMO-{index:04d}-{label}",
                "manager": {"cik": f"DEMO{index:04d}", "name": name, "filer_type": "HA"},
                "report_date": quarter_end(label).isoformat(),
                "filing_date": quarter_end(label).isoformat(),
                "total_aum": aum,
                "holdings": holdings,
                "other_included_count": 0,
            })
    return filings["prior"], filings["current"], data["quarter"]


def main():
    csv_path = Path(__file__).resolve().parent / "holdings_13f.csv"
    if not radar.HAS_NATIVE_ENGINE:
        raise SystemExit(
            "Native engine not built — run: pip install maturin && maturin develop"
        )

    prior, current, current_label = load_filings(csv_path)
    prior_label = prior_quarter(current_label)
    print(
        f"Driving the Rust engine from Python: {prior_label} -> {current_label}, "
        f"{len(prior)} managers\n"
    )

    pipeline = radar.Pipeline()
    result = pipeline.run(prior, current, prior_label, current_label)

    print(result.summary_report())

    top = result.top_bullish_consensus(3)
    print("Top bullish consensus:", [(s["ticker"], round(s["net_direction"], 2)) for s in top])

    # Sanity checks — the engine must see what the CSV encodes.
    assert len(result.diffs) == 2, "expected one diff per manager"
    tickers = {s["ticker"] for s in result.consensus_signals}
    assert {"GOOG", "NVDA", "CP"} <= tickers
    whale_tickers = {m["ticker"] for m in result.whale_moves["moves"]}
    assert "CP" in whale_tickers, "CP was initiated at both funds — must be a whale move"
    assert any(
        d["ticker"] == "NVDA" and d["change"] == "Increased"
        for diff in result.diffs
        for d in diff["diffs"]
    )
    assert any(
        d["ticker"] == "NKE" and d["change"] == "Decreased"
        for diff in result.diffs
        for d in diff["diffs"]
    )
    print("OK: engine-driven analysis matches the sample data.")


if __name__ == "__main__":
    main()
