#!/usr/bin/env python3
"""Merge the base-branch line % into an existing coverage summary JSON.

Best-effort: if the base report is missing or unparseable (the base coverage
step is `continue-on-error`), `base_pct` is left null and the PR comment shows
the delta as "n/a". This never fails the gate.

Usage:
    coverage_merge_base.py <summary.json> <base-report.json>
"""
import json
import os
import sys

# Reuse the shape detection from the sibling summary script.
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from coverage_summary import line_pct  # noqa: E402


def main() -> None:
    if len(sys.argv) != 3:
        raise SystemExit(__doc__)
    summary_path, base_path = sys.argv[1:3]
    with open(summary_path, encoding="utf-8") as fh:
        summary = json.load(fh)
    try:
        with open(base_path, encoding="utf-8") as fh:
            base_doc = json.load(fh)
        summary["base_pct"] = round(line_pct(base_doc), 2)
        print(f"base lines {summary['base_pct']}%")
    except (OSError, ValueError, SystemExit) as err:
        print(f"base coverage unavailable ({err}); delta will render as n/a")
        summary["base_pct"] = None
    with open(summary_path, "w", encoding="utf-8") as fh:
        json.dump(summary, fh, indent=2)


if __name__ == "__main__":
    main()
