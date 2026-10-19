#!/usr/bin/env python3
"""Run a build from verified committed inputs, retaining success/failure evidence."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import time

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("build_source", Path(__file__).with_name("export-build-source.py"))
source = importlib.util.module_from_spec(spec)
spec.loader.exec_module(source)


def run(destination, commit, receipt_path, command, generated=(), cwd=None, environment=None, stdout=None):
    destination = destination.resolve()
    receipt_path = receipt_path.resolve()
    if source.within(receipt_path, destination / "source"):
        raise ValueError("build execution receipt must be outside source inputs")
    receipt_path.parent.mkdir(parents=True, exist_ok=True)
    # A leftover success receipt cannot stand in for this invocation.
    with receipt_path.open("x") as receipt_file:
        exported = source.verify(destination, commit, generated)
        working = destination / "source"
        if cwd:
            working = working / str(source.relative(cwd))
            if not source.within(working.resolve(), destination / "source"):
                raise ValueError("build working directory escapes exported source")
        identity = {"NYXID_GIT_HASH": exported["source"]["commit"],
                    "SOURCE_COMMIT": exported["source"]["commit"],
                    "SOURCE_DATE_EPOCH": str(exported["source"]["epoch"])}
        started = time.monotonic()
        # Inherited Git overrides cannot make an exported build resolve to a
        # different working tree; explicit identity comes only from the receipt.
        env = {**{key: value for key, value in os.environ.items() if not key.startswith("GIT_")},
               **(environment or {}),
               **identity, "GIT_OPTIONAL_LOCKS": "0", "PYTHONDONTWRITEBYTECODE": "1"}
        code, failure = None, None
        try:
            result = subprocess.run(command, cwd=working, env=env, stdout=stdout)
            code = result.returncode
        finally:
            try:
                source.verify(destination, commit, generated)
            except (ValueError, OSError) as error:
                failure = str(error)
            receipt = {
                "schema": "nyxid.exported-build-execution.v1",
                "source": {key: exported["source"][key] for key in ("commit", "epoch")},
                "source_archive": exported["archive"],
                "export_receipt_sha256": hashlib.sha256((destination / "source-export.json").read_bytes()).hexdigest(),
                "command": command, "working_directory": cwd or ".",
                "identity_environment": identity, "generated_paths": list(generated),
                "exit_code": code, "post_build_source_verified": failure is None,
                "post_build_source_error": failure, "seconds": round(time.monotonic() - started, 3),
            }
            json.dump(receipt, receipt_file, indent=2)
            receipt_file.write("\n")
        if failure is not None:
            raise ValueError("build changed authenticated source inputs")
        return code if code >= 0 else 128 - code


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--export", dest="destination", required=True, type=Path)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--receipt", required=True, type=Path)
    parser.add_argument("--generated", action="append", default=[])
    parser.add_argument("--cwd", help="source-relative build directory")
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command
    if command[:1] == ["--"]:
        command = command[1:]
    if not command:
        parser.error("a build command is required")
    raise SystemExit(run(args.destination, args.commit, args.receipt, command, args.generated, args.cwd))


if __name__ == "__main__":
    main()
