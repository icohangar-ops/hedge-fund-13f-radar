"""Basic import tests for hedge-fund-13f-radar.

Validates that core modules can be imported without errors.
"""

def test_import_package():
    """Test that the package imports and exports analyze_13f."""
    from hedge_fund_13f_radar import analyze_13f
    assert callable(analyze_13f)


def test_import_core_dataclasses():
    """Test that core dataclasses are importable."""
    from hedge_fund_13f_radar.core import (
        PositionSignal,
        VerificationGate,
        RadarReport,
    )
    assert PositionSignal is not None
    assert VerificationGate is not None
    assert RadarReport is not None


def test_position_signal_dataclass():
    """Test PositionSignal dataclass creation."""
    from hedge_fund_13f_radar.core import PositionSignal
    sig = PositionSignal(
        manager="Berkshire", ticker="AAPL", company="Apple Inc.",
        sector="Technology", value_usd=1e9, weight=0.15,
        action="HELD", conviction="HIGH",
    )
    assert sig.ticker == "AAPL"
    assert sig.action == "HELD"


def test_verification_gate_dataclass():
    """Test VerificationGate dataclass creation."""
    from hedge_fund_13f_radar.core import VerificationGate
    gate = VerificationGate(status="CLEAR", confidence=100)
    assert gate.status == "CLEAR"
    assert gate.confidence == 100
    assert len(gate.violations) == 0


def test_radar_report_to_dict():
    """Test that RadarReport.to_dict works."""
    from hedge_fund_13f_radar.core import RadarReport, VerificationGate
    report = RadarReport(
        quarter="Q1 2026", consensus_tickers=["AAPL"],
        sector_rotation={"Technology": 1e9},
        position_signals=[], verification=VerificationGate("CLEAR", 100),
    )
    d = report.to_dict()
    assert d["quarter"] == "Q1 2026"
    assert d["consensus_tickers"] == ["AAPL"]


def test_radar_report_to_markdown():
    """Test that RadarReport.to_markdown produces a string."""
    from hedge_fund_13f_radar.core import RadarReport, VerificationGate
    report = RadarReport(
        quarter="Q1 2026", consensus_tickers=["AAPL"],
        sector_rotation={}, position_signals=[],
        verification=VerificationGate("CLEAR", 100),
    )
    md = report.to_markdown()
    assert "Hedge Fund 13F Radar" in md
    assert "Q1 2026" in md


def test_import_report_json():
    """Test that report_json helper imports."""
    from hedge_fund_13f_radar.core import report_json
    assert callable(report_json)
