#!/usr/bin/env python3
"""Evidence for evidence/matrix.yaml C017 (stdlib-only, no network, no install).

README (#compliance-artifacts) claims four CHP compliance artifacts exist in
the repo. This check refuses (exit 1) if any named artifact is missing or
empty. Byte contents are governance-owned and intentionally not hash-pinned.
"""

import sys
from pathlib import Path

ARTIFACTS = [
    ".chp/STATE_MACHINE.md",
    ".chp/R0_CONFIG.yaml",
    ".chp/ADVERSARIAL_PROMPTS.md",
    ".chp/CHP_COMPLIANCE.md",
]


def main() -> int:
    failed = False
    for relpath in ARTIFACTS:
        path = Path(relpath)
        if not path.is_file():
            print(f"FAIL: {relpath} does not exist")
            failed = True
        elif path.stat().st_size == 0:
            print(f"FAIL: {relpath} is empty")
            failed = True
        else:
            print(f"OK: {relpath} present ({path.stat().st_size} bytes)")
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
