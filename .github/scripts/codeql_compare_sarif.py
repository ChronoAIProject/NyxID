#!/usr/bin/env python3
"""Compare CodeQL results after resolving positional SARIF references.

Messages, fingerprints, locations, code flows and referenced rule/taxon
metadata remain part of each result's identity. Only positional references
are replaced with the objects they identify.
"""
from __future__ import annotations

import argparse
import json
from collections import Counter
from pathlib import Path


def normalize_result(result: dict, run: dict) -> dict:
    def component(reference):
        if "index" in reference:
            return run["tool"]["extensions"][reference["index"]]
        return run["tool"]["driver"]

    def visit(value, field=""):
        if isinstance(value, list):
            return [visit(item, field) for item in value]
        if not isinstance(value, dict):
            return value
        value = dict(value)
        if field == "taxa" and "index" in value:
            tool = component(value.get("toolComponent", {}))
            value["definition"] = tool["taxa"][value.pop("index")]
            value["toolComponent"] = {key: tool[key] for key in ("name", "guid", "version") if key in tool}
        elif field == "artifactLocation" and "index" in value:
            location = run["artifacts"][value.pop("index")]["location"]
            for key, item in location.items():
                if key != "index":
                    if key in value and value[key] != item:
                        raise ValueError("conflicting artifact reference")
                    value[key] = item
        elif field == "rule" and "index" in value:
            tool = component(value.get("toolComponent", {}))
            definition = tool["rules"][value.pop("index")]
            if value.get("id", definition["id"]) != definition["id"]:
                raise ValueError("conflicting rule reference")
            value["definition"] = definition
        if "ruleIndex" in value:
            definition = run["tool"]["driver"]["rules"][value.pop("ruleIndex")]
            if value.get("ruleId", definition["id"]) != definition["id"]:
                raise ValueError("conflicting result rule reference")
            value["ruleDefinition"] = definition
        return {key: visit(item, key) for key, item in value.items()}

    return visit(result)


def canonical_results(document: dict) -> Counter:
    if len(document["runs"]) != 1:
        raise ValueError("expected one analysis per file")
    run = document["runs"][0]
    return Counter(json.dumps(normalize_result(result, run), sort_keys=True)
                   for result in run["results"])


def source_uris(value):
    """Locations in the finding, excluding CodeQL rule/model definition files."""
    if isinstance(value, list):
        return set().union(*(source_uris(item) for item in value))
    if isinstance(value, dict):
        found = {value["uri"]} if "uri" in value else set()
        return found.union(*(source_uris(item) for key, item in value.items()
                             if key not in {"definition", "ruleDefinition"}))
    return set()


def compare(baseline: dict, scoped: dict, excluded: set[str]) -> dict:
    before, after = canonical_results(baseline), canonical_results(scoped)
    removed = [json.loads(item) for item in (before - after).elements()]
    added = [json.loads(item) for item in (after - before).elements()]
    outside = [item for item in removed if not source_uris(item) or source_uris(item) - excluded]
    return {"baseline": sum(before.values()), "scoped": sum(after.values()),
            "unchanged": sum((before & after).values()), "removed": removed,
            "added_or_changed": added, "removed_outside_exclusions": outside}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("baseline", type=Path)
    parser.add_argument("scoped", type=Path)
    parser.add_argument("exclusions", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    report = compare(json.loads(args.baseline.read_text()), json.loads(args.scoped.read_text()),
                     set(args.exclusions.read_text().splitlines()))
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({key: len(value) if isinstance(value, list) else value
                      for key, value in report.items()}, sort_keys=True))
    return int(bool(report["added_or_changed"] or report["removed_outside_exclusions"]))


if __name__ == "__main__":
    raise SystemExit(main())
