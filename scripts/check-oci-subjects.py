#!/usr/bin/env python3
"""Inspect bounded OCI metadata; this does not validate signatures or scan layers."""
import argparse
import json
from pathlib import Path

from oci_subjects import directory_reader, inspect
from publication_contract import load_json, source_identity


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--descriptor", type=Path, required=True)
    parser.add_argument("--layout", type=Path, required=True)
    parser.add_argument("--platform", action="append", required=True)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--epoch", type=int, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    source = source_identity(args.commit, args.epoch)
    result = inspect(load_json(args.descriptor), directory_reader(args.layout), args.platform, args.commit)
    result["source"] = source
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print("Validated runnable platforms and bound auxiliary descriptors: " + str(len(result["runnable"])))
