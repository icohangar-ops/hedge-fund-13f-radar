#!/usr/bin/env python3
"""Evidence for evidence/matrix.yaml C010 (stdlib-only, no network, no install).

README (#required-columns) states an exact required-columns list; core.py
defines the REQUIRED_COLUMNS the analysis actually enforces. This check
refuses (exit 1) when the two drift apart — a documented column the code
does not require, or an enforced column the README does not document.
"""

import ast
import re
import sys
from pathlib import Path

README = Path("README.md")
CORE = Path("src/hedge_fund_13f_radar/core.py")


def readme_columns(text: str) -> set | None:
    """The backticked column list under '## Required Columns'."""
    match = re.search(r"## Required Columns\s*\n\s*`([^`]+)`", text)
    if match is None:
        return None
    return {column.strip() for column in match.group(1).split(",")}


def core_required_columns(text: str) -> set | None:
    """The REQUIRED_COLUMNS string-set literal in core.py (via ast — no import,
    so this check runs before any dependency install step)."""
    for node in ast.walk(ast.parse(text)):
        if isinstance(node, ast.Assign):
            for target in node.targets:
                if isinstance(target, ast.Name) and target.id == "REQUIRED_COLUMNS":
                    return {element.value for element in node.value.elts}
    return None


def main() -> int:
    if not README.is_file():
        print(f"FAIL: {README} not found")
        return 1
    if not CORE.is_file():
        print(f"FAIL: {CORE} not found")
        return 1

    documented = readme_columns(README.read_text(encoding="utf-8"))
    if documented is None:
        print("FAIL: no backticked column list found under '## Required Columns' in README.md")
        return 1

    enforced = core_required_columns(CORE.read_text(encoding="utf-8"))
    if enforced is None:
        print(f"FAIL: REQUIRED_COLUMNS set literal not found in {CORE}")
        return 1

    if documented != enforced:
        print(f"FAIL: README documents {sorted(documented)}")
        print(f"      core.REQUIRED_COLUMNS enforces {sorted(enforced)}")
        print(f"      undocumented-but-enforced: {sorted(enforced - documented)}")
        print(f"      documented-but-not-enforced: {sorted(documented - enforced)}")
        return 1

    print(f"OK: README Required Columns match core.REQUIRED_COLUMNS ({len(enforced)} columns)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
