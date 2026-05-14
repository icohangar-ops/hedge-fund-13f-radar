"""CLI for Hedge Fund 13F Radar."""
from __future__ import annotations

import argparse
import json
import sys

from hedge_fund_13f_radar.core import analyze_13f, report_json, store_report


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="hedge-fund-13f-radar")
    sub = parser.add_subparsers(dest="command", required=True)

    # analyze subcommand
    analyze = sub.add_parser("analyze", help="Analyze a normalized 13F holdings CSV.")
    analyze.add_argument("--holdings", required=True)
    analyze.add_argument("--json", action="store_true", dest="as_json")
    analyze.add_argument("--store", action="store_true", help="Persist results to CockroachDB")

    # health subcommand
    sub.add_parser("health", help="Check CockroachDB connectivity")

    args = parser.parse_args(argv)

    if args.command == "analyze":
        report = analyze_13f(args.holdings)
        sys.stdout.write((report_json(report) if args.as_json else report.to_markdown()) + "\n")
        if args.store:
            result = store_report(report)
            sys.stderr.write(json.dumps(result, indent=2) + "\n")
        return 0

    if args.command == "health":
        try:
            from hedge_fund_13f_radar.db.cockroachdb_layer import health_check
            sys.stdout.write(json.dumps(health_check(), indent=2) + "\n")
        except ImportError:
            sys.stderr.write("CockroachDB layer not available\n")
            return 1
        return 0

    return 1


if __name__ == "__main__":
    raise SystemExit(main())
