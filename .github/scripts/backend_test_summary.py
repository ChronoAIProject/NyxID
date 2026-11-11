#!/usr/bin/env python3
"""Preserve failure, retry and timing evidence even when suite verification fails.

This is diagnostic only. backend_shards.py and the execution-job result remain
the gates; an incomplete report must never authorize coverage publication.
"""

import argparse
import html
import json
import math
import os
from pathlib import Path
import xml.etree.ElementTree as ET

from backend_shards import FLAKY_TAGS

# Leave headroom below GitHub's 1 MiB per-step summary limit.
MAX_SUMMARY_BYTES = 900_000


def summarize(results, count):
    cases = []
    unavailable = []
    for shard in range(1, count + 1):
        path = results / f"shard-{shard}" / "junit.xml"
        try:
            root = ET.parse(path).getroot()
        except (OSError, ET.ParseError) as error:
            unavailable.append({"shard": shard, "reason": str(error)})
            continue
        parsed = root.findall(".//testcase")
        if not parsed:
            unavailable.append({"shard": shard, "reason": "empty test report"})
            continue
        for case in parsed:
            tags = {child.tag for child in case}
            # Match the verifier: unknown outcome tags must never look passed.
            failures = tags - FLAKY_TAGS - {"system-out", "system-err", "skipped"}
            status = ("failed" if failures else
                      "skipped" if "skipped" in tags else
                      "retried" if tags & FLAKY_TAGS else "passed")
            try:
                seconds = float(case.get("time", "0"))
                if not math.isfinite(seconds) or seconds < 0:
                    seconds = 0.0
            except ValueError:
                seconds = 0.0
            cases.append({"shard": shard, "binary": case.get("classname", ""),
                          "test": case.get("name", ""), "status": status,
                          "seconds": seconds})
    return {
        "counts": {status: sum(case["status"] == status for case in cases)
                   for status in ("passed", "retried", "failed", "skipped")},
        "unavailable": unavailable,
        "attention": [case for case in cases if case["status"] != "passed"],
        "slowest": sorted(cases, key=lambda case: case["seconds"], reverse=True)[:20],
    }


def markdown(report):
    def label(case):
        # Treat names as text, including fixtures that exercise unusual names.
        name = html.escape(f'{case["binary"]} {case["test"]}').replace("`", "&#96;")
        return f'shard {case["shard"]}: <code>{name}</code>'

    lines = ["### Backend test diagnostics", "",
             "Diagnostic evidence only; complete-suite verification is a separate gate.", "",
             ", ".join(f"{value} {key}" for key, value in report["counts"].items()), ""]
    for missing in report["unavailable"]:
        lines.append(f'- Shard {missing["shard"]}: JUnit report unavailable, malformed or empty.')
    for case in report["attention"]:
        lines.append(f'- **{case["status"]}** — {label(case)}')
    lines.extend(["", "Slowest reported test cases (JUnit durations):", ""])
    for case in report["slowest"]:
        lines.append(f'- {case["seconds"]:.3f}s — {label(case)}')
    rendered = "\n".join(lines) + "\n"
    encoded = rendered.encode("utf-8")
    if len(encoded) > MAX_SUMMARY_BYTES:
        notice = ("\n\nSummary truncated to fit GitHub's size limit. "
                  "Complete results remain in the JSON and JUnit artifacts.\n")
        prefix = encoded[:MAX_SUMMARY_BYTES - len(notice.encode("utf-8"))]
        # Keep whole lines so neither UTF-8 characters nor HTML tags are split.
        return prefix.rsplit(b"\n", 1)[0].decode("utf-8") + notice
    return rendered


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("results", type=Path)
    parser.add_argument("--count", type=int, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.count < 1:
        parser.error("--count must be positive")
    report = summarize(args.results, args.count)
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    rendered = markdown(report)
    print(rendered)
    if summary := os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(summary, "a", encoding="utf-8") as handle:
            handle.write(rendered)


if __name__ == "__main__":
    main()
