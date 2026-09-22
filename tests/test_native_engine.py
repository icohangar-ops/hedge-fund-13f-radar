"""Integration tests for the native Rust engine bindings (pyo3).

Mirrors the core Rust test paths from `src/lib.rs` (integration_tests) and the
module-level unit tests in `src/ingest.rs`, `src/types.rs` and `src/pipeline.rs`:
the same scenarios, the same assertions, driven from Python through
`hedge_fund_13f_radar._native`.

Requires the native module to be built first: `pip install maturin && maturin develop`.
"""

from datetime import date

import pytest

radar = pytest.importorskip(
    "hedge_fund_13f_radar._native",
    reason="native engine not built — run `pip install maturin && maturin develop`",
)

# ---------------------------------------------------------------------------
# Test data — mirrors TestDataBuilder + make_fund_pair in src/lib.rs
# ---------------------------------------------------------------------------

CUSIP_SEEDS = [  # TestDataBuilder::new()
    ("037833100", "AAPL", "Apple Inc"),
    ("478160104", "JNJ", "Johnson & Johnson"),
    ("02079K305", "GOOGL", "Alphabet Inc"),
    ("594918104", "MSFT", "Microsoft Corp"),
    ("46647Q103", "JPM", "JPMorgan Chase"),
    ("30303M102", "META", "Meta Platforms"),
    ("88160R101", "TSLA", "Tesla Inc"),
    ("92343V104", "NVDA", "NVIDIA Corp"),
    ("580135101", "MA", "Mastercard Inc"),
    ("949746101", "UNH", "UnitedHealth Group"),
    ("54615U100", "LLY", "Eli Lilly & Co"),
]


def make_cusip_map():
    cusips = radar.CusipMap()
    for cusip, ticker, name in CUSIP_SEEDS:
        cusips.insert(cusip, ticker, name)
    return cusips


def default_config():  # TestDataBuilder::default_config
    return {
        "cusip_map": make_cusip_map(),
        "min_conviction_holders": 2,
        "min_avg_conviction": 0.5,
        "whale_threshold_m": 0.01,
        "pair_trade_min_spread": 1.0,
        "pair_trade_min_rotating": 1,
        "cluster_similarity": 0.2,
    }


def holding(ticker, shares, value, weight):
    return {
        "cusip": f"{ticker}_CUSIP",
        "ticker": ticker,
        "name": f"{ticker} Corp",
        "sector": radar.classify_ticker(ticker),
        "shares": shares,
        "value": value,
        "share_type": "SH",
        "discretion": "SO",
        "is_option": False,
        "portfolio_weight": weight,
    }


PRIOR_HOLDINGS = [
    holding("AAPL", 1000, 500_000.0, 0.3),
    holding("MSFT", 800, 400_000.0, 0.24),
    holding("JNJ", 500, 300_000.0, 0.18),
    holding("JPM", 400, 200_000.0, 0.12),
    holding("UNH", 200, 120_000.0, 0.07),
    holding("XOM", 300, 150_000.0, 0.09),
]

CURRENT_HOLDINGS = [
    holding("AAPL", 1400, 700_000.0, 0.35),
    holding("MSFT", 1000, 500_000.0, 0.25),
    holding("NVDA", 200, 300_000.0, 0.15),
    holding("JPM", 300, 150_000.0, 0.075),
    holding("JNJ", 300, 180_000.0, 0.09),
    holding("UNH", 250, 150_000.0, 0.075),
]


def filing(cik, name, holdings, report_date, aum):
    return {
        "accession_number": f"{cik}_{report_date}",
        "manager": {"cik": cik, "name": name, "filer_type": "HA"},
        "report_date": report_date.isoformat(),
        "filing_date": report_date.isoformat(),
        "total_aum": aum,
        "holdings": holdings,
        "other_included_count": 0,
    }


def make_fund_pair(cik, name, shift=0):  # make_fund_pair in src/lib.rs
    def shifted(holdings):
        out = [dict(h) for h in holdings]
        out[0]["shares"] += shift  # shift applies to AAPL in both quarters
        return out

    prior = shifted(PRIOR_HOLDINGS)
    current = shifted(CURRENT_HOLDINGS)
    prior_aum = sum(h["value"] for h in prior)
    current_aum = sum(h["value"] for h in current)
    return (
        filing(cik, name, prior, date(2024, 3, 31), prior_aum),
        filing(cik, name, current, date(2024, 6, 30), current_aum),
    )


def make_pipeline():
    return radar.Pipeline.with_config(default_config())


def find(items, key, value):
    return next((item for item in items if item[key] == value), None)


# ---------------------------------------------------------------------------
# Pipeline integration paths (mirrors src/lib.rs integration tests)
# ---------------------------------------------------------------------------


def test_full_pipeline_multi_manager():
    pipeline = make_pipeline()
    funds = ["Viking Global", "Bridgewater", "Third Point", "Greenlight", "Citadel"]
    pairs = [make_fund_pair(f"C{i:04d}", fund, i * 100) for i, fund in enumerate(funds)]
    priors = [p for p, _ in pairs]
    currents = [c for _, c in pairs]

    result = pipeline.run(priors, currents, "2024-Q1", "2024-Q2")

    assert len(result.diffs) == 5
    assert result.consensus_signals
    assert result.high_conviction
    assert result.sector_momentum
    assert result.rotation_metrics
    assert result.clusters

    aapl_hc = find(result.high_conviction, "ticker", "AAPL")
    assert aapl_hc is not None
    assert aapl_hc["holder_count"] >= 5

    aapl_signal = find(result.consensus_signals, "ticker", "AAPL")
    assert aapl_signal is not None
    assert aapl_signal["net_direction"] > 0.0


def test_whale_detection():
    pipeline = make_pipeline()
    p1, c1 = make_fund_pair("C0001", "Large Fund")
    p2, c2 = make_fund_pair("C0002", "Small Fund")

    result = pipeline.run([p1, p2], [c1, c2], "2024-Q1", "2024-Q2")

    assert len(result.whale_moves["moves"]) > 0


def test_sector_rotation():
    pipeline = make_pipeline()
    funds = ["Tech Fund", "Health Fund", "Macro Fund"]
    pairs = [make_fund_pair(f"C{i:04d}", fund) for i, fund in enumerate(funds)]
    priors = [p for p, _ in pairs]
    currents = [c for _, c in pairs]

    result = pipeline.run(priors, currents, "2024-Q1", "2024-Q2")

    tech = find(result.sector_momentum, "sector", "Technology")
    assert tech is not None
    assert tech["score"] > 0.0

    energy = find(result.sector_momentum, "sector", "Energy")
    assert energy is not None
    assert energy["score"] < 0.0


def test_with_aggregator():
    pipeline = make_pipeline()
    agg = radar.QuarterlyAggregator()
    for i, fund in enumerate(["Fund A", "Fund B", "Fund C"]):
        prior, current = make_fund_pair(f"C{i:04d}", fund)
        agg.add(prior)
        agg.add(current)

    assert len(agg) == 6
    result = pipeline.run_from_aggregator(agg, "2024-Q1", "2024-Q2")
    assert len(result.diffs) == 3


def test_report_generation():
    pipeline = make_pipeline()
    p1, c1 = make_fund_pair("C0001", "Alpha")
    p2, c2 = make_fund_pair("C0002", "Beta")

    result = pipeline.run([p1, p2], [c1, c2], "2024-Q1", "2024-Q2")
    report = result.summary_report()

    assert "13F Radar Report" in report
    assert "Managers analyzed: 2" in report
    assert "Sector Momentum" in report
    assert "High Conviction Tickers" in report


def test_pipeline_new_manager_warning():  # mirrors pipeline.rs test
    pipeline = make_pipeline()
    _, c1 = make_fund_pair("CNEW", "New Fund")

    result = pipeline.run([], [c1], "2024-Q1", "2024-Q2")

    assert result.warnings
    assert any("no prior quarter" in w for w in result.warnings)


def test_missing_quarter_error():  # mirrors pipeline.rs test
    pipeline = make_pipeline()
    agg = radar.QuarterlyAggregator()

    with pytest.raises(ValueError):
        pipeline.run_from_aggregator(agg, "2024-Q1", "2024-Q2")


def test_top_bullish_and_heat_map():  # mirrors test_pipeline_top_bullish + heat map
    pipeline = make_pipeline()
    p1, c1 = make_fund_pair("C1", "Alpha")
    result = pipeline.run([p1], [c1], "2024-Q1", "2024-Q2")

    top = result.top_bullish_consensus(3)
    assert 0 < len(top) <= 3
    assert all(s["net_direction"] >= 0.0 for s in top)

    top_in = result.top_inflow_sectors(3)
    top_out = result.top_outflow_sectors(3)
    assert top_in and top_out

    assert result.heat_map["net_flows"]
    assert result.to_dict()["prior_quarter"] == "2024-Q1"
    assert str(result).startswith("═")


# ---------------------------------------------------------------------------
# Ingest paths (mirrors src/ingest.rs tests)
# ---------------------------------------------------------------------------

CUSIP_MAP = make_cusip_map()


def test_parse_csv_holding():
    raw = radar.parse_csv_holding("037833100,AAPL,Apple Inc,1000,500.0,SH,SO,false")
    assert raw["cusip"] == "037833100"
    assert raw["ticker"] == "AAPL"
    assert raw["shares"] == 1000
    assert abs(raw["value"] - 500_000.0) < 1.0
    assert raw["is_option"] is False


def test_parse_csv_option():
    raw = radar.parse_csv_holding("037833100,AAPL,Apple Inc,500,200.0,PUT,DS,true")
    assert raw["is_option"] is True


def test_parse_csv_too_few_cols():
    with pytest.raises(ValueError):
        radar.parse_csv_holding("037833100")


def test_parse_xml_holding():
    xml = """
        <infoTable>
            <cusip>037833100</cusip>
            <ticker>AAPL</ticker>
            <nameOfIssuer>Apple Inc</nameOfIssuer>
            <sshPrnamt>1000</sshPrnamt>
            <value>500</value>
            <sshPrmType>SH</sshPrmType>
            <investmentDiscretion>SO</investmentDiscretion>
        </infoTable>
    """
    raw = radar.parse_xml_holding(xml)
    assert raw["cusip"] == "037833100"
    assert raw["ticker"] == "AAPL"
    assert raw["shares"] == 1000
    assert abs(raw["value"] - 500_000.0) < 1.0


def test_parse_xml_missing_cusip():
    with pytest.raises(ValueError):
        radar.parse_xml_holding("<infoTable><ticker>AAPL</ticker></infoTable>")


def test_normalize_holding():
    holding_dict = radar.normalize_holding(
        {
            "cusip": "037833100",
            "ticker": "AAPL",
            "name": "Apple Inc",
            "shares": 1000,
            "value": 500_000.0,
        },
        CUSIP_MAP,
        1_000_000.0,
    )
    assert holding_dict["ticker"] == "AAPL"
    assert holding_dict["sector"] == "Technology"
    assert abs(holding_dict["portfolio_weight"] - 0.5) < 1e-9


def test_normalize_holding_negative_value():
    with pytest.raises(ValueError):
        radar.normalize_holding(
            {"cusip": "037833100", "ticker": "AAPL", "shares": 1000, "value": -500.0},
            CUSIP_MAP,
            1_000_000.0,
        )


def test_normalize_holding_cusip_fallback():
    holding_dict = radar.normalize_holding(
        {"cusip": "037833100", "shares": 1000, "value": 500_000.0},
        CUSIP_MAP,
        1_000_000.0,
    )
    assert holding_dict["ticker"] == "AAPL"
    assert holding_dict["name"] == "Apple Inc"


def test_filing_builder():
    builder = radar.FilingBuilder(
        {"cik": "0001234567", "name": "Test Capital", "filer_type": "HA"},
        date(2024, 3, 31),
    )
    builder.accession("000000000000-01-000001")
    builder.total_aum(1_000_000.0)
    builder.add_csv_holding("037833100,AAPL,Apple Inc,1000,500.0,SH,SO,false")
    builder.add_csv_holding("478160104,JNJ,Johnson & Johnson,500,500.0,SH,SO,false")

    filing_dict = builder.build(CUSIP_MAP)
    assert len(filing_dict["holdings"]) == 2
    assert filing_dict["total_aum"] == 1_000_000.0
    assert filing_dict["report_date"] == "2024-03-31"

    with pytest.raises(ValueError):  # builder consumed
        builder.total_aum(1.0)


def test_filing_builder_duplicate_cusip():
    builder = radar.FilingBuilder(
        {"cik": "0001234567", "name": "Test Capital", "filer_type": "HA"},
        date(2024, 3, 31),
    )
    builder.total_aum(1_000_000.0)
    builder.add_csv_holding("037833100,AAPL,Apple Inc,1000,500.0,SH,SO,false")
    builder.add_csv_holding("037833100,AAPL,Apple Inc,1000,500.0,SH,SO,false")

    with pytest.raises(ValueError, match="Duplicate CUSIP"):
        builder.build(CUSIP_MAP)


def test_quarterly_aggregator():
    agg = radar.QuarterlyAggregator()
    assert agg.is_empty()

    def empty_filing(name, report_date):
        return filing(f"C{name}", name, [], date(*report_date), 100_000.0)

    agg.add(empty_filing("A", (2024, 3, 31)))
    agg.add(empty_filing("B", (2024, 3, 31)))
    agg.add(empty_filing("C", (2024, 6, 30)))

    assert len(agg) == 3
    by_quarter = agg.by_quarter()
    assert len(by_quarter["2024-Q1"]) == 2
    assert len(by_quarter["2024-Q2"]) == 1
    assert agg.latest_quarter() == "2024-Q2"
    assert len(agg.filings_for_quarter("2024-Q2")) == 1


def test_validate_filing():
    _, current = make_fund_pair("C1", "Alpha")
    assert radar.validate_filing(current) == []

    empty = filing("C1", "Empty Fund", [], date(2024, 3, 31), 0.0)
    warnings = radar.validate_filing(empty)
    assert warnings


def test_parse_date():
    assert radar.parse_date("2024-03-31") == date(2024, 3, 31)
    with pytest.raises(ValueError):
        radar.parse_date("not-a-date")


# ---------------------------------------------------------------------------
# Types paths (mirrors src/types.rs tests)
# ---------------------------------------------------------------------------


def test_classify_ticker():
    assert radar.classify_ticker("AAPL") == "Technology"
    assert radar.classify_ticker("aapl") == "Technology"
    assert radar.classify_ticker("JNJ") == "Healthcare"
    assert radar.classify_ticker("XOM") == "Energy"
    assert radar.classify_ticker("ZZZZ") == "Unknown"


def test_conviction_from_portfolio_weight():
    assert radar.conviction_from_portfolio_weight(0.005) == "Low"
    assert radar.conviction_from_portfolio_weight(0.02) == "Medium"
    assert radar.conviction_from_portfolio_weight(0.04) == "High"
    assert radar.conviction_from_portfolio_weight(0.08) == "VeryHigh"


def test_cusip_map_insert_lookup():
    cusips = radar.CusipMap()
    assert cusips.is_empty()

    cusips.insert("037833100", "AAPL", "Apple Inc")
    assert len(cusips) == 1
    assert cusips.get("037833100") == ("AAPL", "Apple Inc")
    assert cusips.get("000000000") is None


# ---------------------------------------------------------------------------
# Diff paths (mirrors src/diff.rs behaviours)
# ---------------------------------------------------------------------------


def test_diff_filings():
    prior, current = make_fund_pair("C1", "Alpha")
    diff = radar.diff_filings(prior, current)

    assert diff["manager_name"] == "Alpha"
    changes = {d["ticker"]: d["change"] for d in diff["diffs"]}
    assert changes["AAPL"] == "Increased"
    assert changes["NVDA"] == "New"
    assert changes["XOM"] == "Exited"

    assert diff["summary"]["new_count"] == 1
    assert diff["summary"]["increased_count"] == 3
    assert diff["summary"]["decreased_count"] == 2
    assert diff["summary"]["exited_count"] == 1

    aapl = find(diff["diffs"], "ticker", "AAPL")
    assert aapl["delta_shares"] == 400
    assert abs(aapl["delta_value"] - 200_000.0) < 1e-9
    assert not aapl["is_exit"]


# ---------------------------------------------------------------------------
# Package surface
# ---------------------------------------------------------------------------


def test_package_reexports_native_engine():
    import hedge_fund_13f_radar

    assert hedge_fund_13f_radar.HAS_NATIVE_ENGINE is True
    assert hedge_fund_13f_radar.Pipeline is radar.Pipeline
    assert "Pipeline" in hedge_fund_13f_radar.__all__


def test_python_demo_end_to_end():
    """The README's claimed walk-through (`python examples/python_demo.py`)
    must run end-to-end against the built bindings: sample CSV in, engine
    analysis out, exit 0, sanity assertions inside the demo passing."""
    import os
    import subprocess
    import sys
    from pathlib import Path

    repo_root = Path(__file__).resolve().parents[1]
    proc = subprocess.run(
        [sys.executable, "examples/python_demo.py"],
        cwd=str(repo_root),
        env=dict(os.environ, PYTHONPATH=str(repo_root / "src")),
        capture_output=True,
        text=True,
        timeout=120,
    )
    assert proc.returncode == 0, proc.stderr
    assert "OK: engine-driven analysis matches the sample data." in proc.stdout
