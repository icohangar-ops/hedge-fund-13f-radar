"""Evidence for evidence/matrix.yaml C009: the README's stated gate triggers.

README (#required-columns): "The verifier returns REQUIRES_HUMAN_VERIFICATION
if source URLs are missing, numeric fields are invalid, or the file does not
contain enough managers for cross-fund analysis." Each trigger gets a focused
deterministic test against core._verify.
"""

from hedge_fund_13f_radar.core import _verify


def valid_rows():
    """Two managers, all required columns populated, valid numerics."""
    base = {
        "manager": "",
        "quarter": "2026Q1",
        "ticker": "GOOG",
        "company": "Alphabet",
        "sector": "Communication Services",
        "shares": "100",
        "prior_shares": "80",
        "value_usd": "12000000",
        "source_url": "",
    }
    row_a = {**base, "manager": "Fund A", "source_url": "https://example.com/fund-a"}
    row_b = {**base, "manager": "Fund B", "source_url": "https://example.com/fund-b"}
    return [row_a, row_b]


def test_gate_flags_missing_source_urls():
    rows = valid_rows()
    rows[0]["source_url"] = ""
    gate = _verify(rows)
    assert gate.status == "REQUIRES_HUMAN_VERIFICATION"
    assert any("source_url" in violation for violation in gate.violations)


def test_gate_flags_invalid_numeric_fields():
    rows = valid_rows()
    rows[1]["shares"] = "many"
    gate = _verify(rows)
    assert gate.status == "REQUIRES_HUMAN_VERIFICATION"
    assert any("invalid numeric field: shares" in violation for violation in gate.violations)


def test_gate_flags_insufficient_managers_for_cross_fund():
    rows = valid_rows()
    rows[1]["manager"] = "Fund A"  # collapse to a single manager
    gate = _verify(rows)
    assert gate.status == "REQUIRES_HUMAN_VERIFICATION"
    assert any("at least two managers" in violation for violation in gate.violations)
