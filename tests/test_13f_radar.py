from hedge_fund_13f_radar.core import analyze_13f


def test_13f_radar_detects_initiations_and_consensus():
    report = analyze_13f("examples/holdings_13f.csv")
    assert report.verification.status == "CLEAR"
    assert "GOOG" in report.consensus_tickers
    assert "CP" in report.consensus_tickers
    assert any(signal.action == "INITIATED" for signal in report.position_signals)
    assert report.sector_rotation

