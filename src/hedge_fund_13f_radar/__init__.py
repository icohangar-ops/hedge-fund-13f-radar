"""Hedge Fund 13F Radar package.

Two surfaces live here:

- ``analyze_13f`` — the pure-Python analysis workflow (no Rust toolchain
  needed to run, once installed).
- The Rust engine bindings (pyo3) — ``FilingBuilder``, ``QuarterlyAggregator``,
  ``Pipeline``, ``diff_filings`` and friends, compiled by maturin from
  ``src/*.rs`` into the ``hedge_fund_13f_radar._native`` module. They are
  re-exported below whenever the native module has been built (``maturin
  develop``); check ``HAS_NATIVE_ENGINE`` to see which surface is live.
"""

from hedge_fund_13f_radar.core import analyze_13f

__all__ = ["analyze_13f"]

try:
    from hedge_fund_13f_radar._native import (  # noqa: F401
        CusipMap,
        FilingBuilder,
        Pipeline,
        PipelineResult,
        QuarterlyAggregator,
        classify_ticker,
        conviction_from_portfolio_weight,
        diff_filings,
        normalize_holding,
        parse_csv_holding,
        parse_date,
        parse_xml_holding,
        validate_filing,
    )

    HAS_NATIVE_ENGINE = True
    __all__ += [
        "CusipMap",
        "FilingBuilder",
        "Pipeline",
        "PipelineResult",
        "QuarterlyAggregator",
        "classify_ticker",
        "conviction_from_portfolio_weight",
        "diff_filings",
        "normalize_holding",
        "parse_csv_holding",
        "parse_date",
        "parse_xml_holding",
        "validate_filing",
        "HAS_NATIVE_ENGINE",
    ]
except ImportError:
    # Native module not built (plain setuptools/pure-Python install).
    HAS_NATIVE_ENGINE = False

