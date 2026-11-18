#!/usr/bin/env python3
"""Require a complete, disjoint, passing nextest run before merging coverage."""

import argparse
import json
import os
from pathlib import Path
import shutil
import xml.etree.ElementTree as ET

FLAKY_TAGS = {"flakyFailure", "flakyError"}


def verify_shard_directories(results, count):
    expected = {f"shard-{shard}" for shard in range(1, count + 1)}
    actual = {path.name for path in results.iterdir() if path.is_dir()}
    if count < 1 or actual != expected:
        raise ValueError(f"shard directory mismatch: expected={sorted(expected)}, "
                         f"actual={sorted(actual)}")


def verify(inventory, results, count, output):
    verify_shard_directories(results, count)
    expected = set()
    ignored = 0
    suites = json.loads(inventory.read_text())["rust-suites"]
    for binary, suite in suites.items():
        if suite["status"] != "listed":
            raise ValueError(f"test binary was not listed: {binary}")
        for name, case in suite["testcases"].items():
            if case["filter-match"]["status"] == "matches":
                expected.add((binary, name))
            elif case["ignored"] and case["filter-match"]["reason"] == "ignored":
                ignored += 1
            else:
                raise ValueError(f"unexpected inventory exclusion: {binary} {name}")
    if not expected:
        raise ValueError("the full test inventory is empty")

    actual = set()
    flaky = []
    combined = ET.Element("testsuites", name="backend-shards")
    for shard in range(1, count + 1):
        report = results / f"shard-{shard}" / "junit.xml"
        root = ET.parse(report).getroot()
        cases = root.findall(".//testcase")
        if not cases:
            raise ValueError(f"empty shard report: {report}")
        for case in cases:
            key = (case.attrib["classname"], case.attrib["name"])
            tags = {child.tag for child in case} - {"system-out", "system-err"}
            # nextest records a retried-then-passed attempt as flakyFailure /
            # flakyError. That test passed and is reported by name below; any
            # other child (failure, error, skipped) is a non-passing test.
            if tags - FLAKY_TAGS:
                raise ValueError(f"non-passing test: {key}")
            if tags:
                flaky.append((shard, key))
            if key in actual:
                raise ValueError(f"test ran more than once: {key}")
            actual.add(key)
        for suite in root.findall("testsuite"):
            combined.append(suite)
    if actual != expected:
        missing = sorted(expected - actual)
        extra = sorted(actual - expected)
        raise ValueError(f"test inventory mismatch: missing={missing[:10]}, extra={extra[:10]}")

    combined.set("tests", str(len(actual)))
    for key in ("failures", "errors", "skipped"):
        combined.set(key, "0")
    output.parent.mkdir(parents=True, exist_ok=True)
    ET.ElementTree(combined).write(output, encoding="utf-8", xml_declaration=True)
    print(f"Verified {len(actual)} tests exactly once across {count} shards; "
          f"{ignored} pre-existing ignored tests; {len(flaky)} passed only on retry.")
    report_flaky(flaky)
    return flaky


def report_flaky(flaky):
    """Keep every retried test visible: stdout plus the GitHub job summary."""
    if not flaky:
        return
    lines = [f"- shard {shard}: `{binary}` `{name}`" for shard, (binary, name) in sorted(flaky)]
    print("Tests that passed only on retry (first attempt failed):")
    print("\n".join(lines))
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a", encoding="utf-8") as handle:
            handle.write(f"### {len(flaky)} test(s) passed only on retry\n\n" + "\n".join(lines) + "\n")


def stage_profiles(results, count, destination):
    verify_shard_directories(results, count)
    # Keep every shard's counters. The %4m filenames are identical between
    # runners; flattening artifact downloads would silently overwrite profiles.
    sources = []
    for shard in range(1, count + 1):
        profiles = sorted((results / f"shard-{shard}").glob("*.profraw"))
        if not profiles or any(path.stat().st_size == 0 for path in profiles):
            raise ValueError(f"missing or empty coverage profiles for shard {shard}")
        sources.extend((shard, path) for path in profiles)
    destination.mkdir(parents=True, exist_ok=True)
    if any(destination.glob("*.profraw")):
        raise ValueError("coverage destination already contains profiles")
    for shard, source in sources:
        shutil.copyfile(source, destination / f"shard-{shard}-{source.name}")
    print(f"Staged {len(sources)} coverage profiles from all {count} shards.")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("inventory", type=Path)
    parser.add_argument("results", type=Path)
    parser.add_argument("--count", type=int, required=True)
    parser.add_argument("--output", type=Path, default=Path("target/nextest/ci/junit.xml"))
    parser.add_argument("--profiles", type=Path)
    args = parser.parse_args()
    if args.count < 1:
        parser.error("--count must be positive")
    verify(args.inventory, args.results, args.count, args.output)
    if args.profiles is not None:
        stage_profiles(args.results, args.count, args.profiles)


if __name__ == "__main__":
    main()
