"""CLI for Hedge Fund 13F Radar."""
from __future__ import annotations

import argparse
import sys

from hedge_fund_13f_radar.core import analyze_13f, report_json


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="hedge-fund-13f-radar")
    sub = parser.add_subparsers(dest="command", required=True)
    analyze = sub.add_parser("analyze", help="Analyze a normalized 13F holdings CSV.")
    analyze.add_argument("--holdings", required=True)
    analyze.add_argument("--json", action="store_true")
    args = parser.parse_args(argv)

    if args.command == "analyze":
        report = analyze_13f(args.holdings)
        sys.stdout.write((report_json(report) if args.json else report.to_markdown()) + "\n")
        return 0
    return 1


if __name__ == "__main__":
    raise SystemExit(main())

