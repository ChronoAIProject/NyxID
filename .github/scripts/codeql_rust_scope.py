#!/usr/bin/env python3
"""Select CodeQL languages conservatively and prepare the Rust PR scope."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path

import yaml

from codeql_cfg_test_files import analyze_tree

WORKFLOW_FILE = ".github/workflows/codeql.yml"
LANGUAGES = ["actions", "javascript-typescript", "python", "rust"]


def select_languages(event: str, outcome: str, rust_changed: str) -> list[str]:
    """Only a successful, explicit no-Rust PR diff can omit Rust."""
    if event == "pull_request" and outcome == "success" and rust_changed == "false":
        return LANGUAGES[:-1]
    return list(LANGUAGES)


def prepare_config(root: Path, event: str, destination: Path) -> list[str]:
    """Append only module-tree-proven test files; keep every base query setting."""
    config = yaml.safe_load((root / ".github/codeql/codeql-config.yml").read_text())
    if not isinstance(config, dict):
        raise ValueError("CodeQL base config must be a mapping")
    excluded = []
    if event == "pull_request":
        result = analyze_tree(root)
        excluded = result.excluded()
        print(json.dumps({"excluded_count": len(excluded), "poisoned": result.poisoned,
                          "notes": result.notes}, sort_keys=True))
        config["paths-ignore"] = [*config.get("paths-ignore", []), *excluded]
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_text(yaml.safe_dump(config, sort_keys=False), encoding="utf-8")
    destination.with_suffix(".excluded.txt").write_text(
        "".join(f"{path}\n" for path in excluded), encoding="utf-8"
    )
    return excluded


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    config = commands.add_parser("config")
    config.add_argument("--root", type=Path, default=Path.cwd())
    config.add_argument("--event", required=True)
    config.add_argument("--output", type=Path, required=True)
    commands.add_parser("matrix")
    args = parser.parse_args()
    if args.command == "config":
        prepare_config(args.root, args.event, args.output)
    else:
        languages = select_languages(os.environ.get("CODEQL_EVENT", ""),
                                     os.environ.get("DETECTION_OUTCOME", ""),
                                     os.environ.get("RUST_CHANGED", ""))
        value = json.dumps(languages, separators=(",", ":"))
        with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as output:
            output.write(f"languages={value}\n")
        print(f"CodeQL languages: {value}")


if __name__ == "__main__":
    main()
