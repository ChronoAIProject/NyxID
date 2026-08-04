#!/usr/bin/env python3
"""Render the aggregated coverage PR comment from per-component summary JSONs.

Reads every coverage-*.json produced by the coverage jobs (issue #785) in the
given directory and prints a markdown table to stdout: each component's head
line %, its enforced threshold, pass/fail status, and the delta vs. the base
branch. Components whose job was skipped (e.g. docs-only PR) simply don't have
a summary file and are omitted.

Usage:
    coverage_comment.py <artifacts-dir>
"""
import glob
import json
import os
import sys

MARKER = "<!-- nyxid-coverage-report -->"

# Stable display order regardless of artifact download order.
ORDER = {"backend": 0, "cli": 1, "frontend": 2}


def fmt_delta(head: float, base) -> str:
    if base is None:
        return "n/a"
    d = round(head - base, 2)
    if d > 0:
        return f"🔺 +{d:.2f}"
    if d < 0:
        return f"🔻 {d:.2f}"
    return "—  0.00"


def main() -> None:
    if len(sys.argv) != 2:
        raise SystemExit(__doc__)
    root = sys.argv[1]
    summaries = []
    for path in glob.glob(os.path.join(root, "**", "coverage-*.json"), recursive=True):
        # Skip the raw/base intermediates; only the normalized summaries have
        # both "component" and "threshold".
        try:
            with open(path, encoding="utf-8") as fh:
                doc = json.load(fh)
        except (OSError, ValueError):
            continue
        if "component" in doc and "threshold" in doc and "head_pct" in doc:
            summaries.append(doc)

    # De-dup by component (merge-multiple download can surface one file each).
    by_component = {}
    for s in summaries:
        by_component[s["component"]] = s
    summaries = sorted(by_component.values(), key=lambda s: ORDER.get(s["component"], 99))

    lines = [
        MARKER,
        "## 📊 Code coverage",
        "",
    ]
    if not summaries:
        lines.append("_No coverage components ran for this PR._")
        print("\n".join(lines))
        return

    lines += [
        "| Component | Lines | Threshold | Status | Δ vs base |",
        "| --- | ---: | ---: | :---: | ---: |",
    ]
    any_fail = False
    for s in summaries:
        head = s["head_pct"]
        threshold = s["threshold"]
        passed = head >= threshold
        any_fail = any_fail or not passed
        status = "✅" if passed else "❌"
        lines.append(
            f"| {s['label']} | {head:.2f}% | {threshold}% | {status} | "
            f"{fmt_delta(head, s.get('base_pct'))} |"
        )

    lines += [
        "",
        "Gate: line coverage must stay at or above the threshold. "
        "Ratchet plan (W21): Backend → 55%, CLI → 50%, Frontend → 30% by quarter end.",
    ]
    if any_fail:
        lines.append("")
        lines.append("> ❌ One or more components are below threshold — see the failing coverage job.")

    print("\n".join(lines))


if __name__ == "__main__":
    main()
