#!/usr/bin/env python3
"""Normalize a coverage report into a tiny summary JSON for the CI gate.

Used by .github/workflows/ci.yml coverage jobs (issue #785). Reads either a
`cargo llvm-cov report --json --summary-only` document (Rust) or a vitest v8
`coverage-summary.json` (Frontend) and writes:

    {"component": "...", "label": "...", "threshold": <int>, "head_pct": <float>}

The base-branch line % is merged in later by coverage_merge_base.py.

Usage:
    coverage_summary.py <component> <label> <threshold> <input.json> <output.json>
"""
import json
import sys


def line_pct(doc: dict) -> float:
    """Extract total line coverage % from either supported report shape."""
    # cargo-llvm-cov: {"data": [{"totals": {"lines": {"percent": ...}}}]}
    data = doc.get("data")
    if isinstance(data, list) and data:
        totals = data[0].get("totals", {})
        lines = totals.get("lines", {})
        if "percent" in lines:
            return float(lines["percent"])
    # vitest v8 json-summary: {"total": {"lines": {"pct": ...}}}. v8 emits the
    # string "Unknown" when no files match the include glob — treat that (and
    # any non-numeric value) as 0% so the gate fails loudly instead of crashing.
    total = doc.get("total")
    if isinstance(total, dict):
        lines = total.get("lines", {})
        if "pct" in lines:
            try:
                return float(lines["pct"])
            except (TypeError, ValueError):
                return 0.0
    raise SystemExit(f"unrecognized coverage report shape; keys={list(doc)}")


def main() -> None:
    if len(sys.argv) != 6:
        raise SystemExit(__doc__)
    component, label, threshold, in_path, out_path = sys.argv[1:6]
    with open(in_path, encoding="utf-8") as fh:
        doc = json.load(fh)
    summary = {
        "component": component,
        "label": label,
        "threshold": int(threshold),
        "head_pct": round(line_pct(doc), 2),
        "base_pct": None,
    }
    with open(out_path, "w", encoding="utf-8") as fh:
        json.dump(summary, fh, indent=2)
    print(f"{label}: head lines {summary['head_pct']}% (threshold {threshold}%)")


if __name__ == "__main__":
    main()
