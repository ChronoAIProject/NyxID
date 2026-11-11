#!/usr/bin/env python3
"""Retry a compiler command once after a stale Rust incremental failure.

The retry is deliberately narrow: ordinary compiler errors and test failures
are returned unchanged. The command's combined output is streamed to stdout
and copied to a log so the marker check works without hiding CI diagnostics.
"""

import argparse
from pathlib import Path
import shutil
import subprocess
import sys


RETRY_MARKERS = (
    "internal compiler error",
    "incremental compilation",
    "failed to load dep graph",
    "dep-graph",
)
DEFAULT_INCREMENTAL_DIRS = (
    Path("target/debug/incremental"),
    Path("target/llvm-cov-target/debug/incremental"),
)


def should_retry(output):
    """Return whether compiler output identifies a stale incremental failure."""
    lowered = output.lower()
    return any(marker in lowered for marker in RETRY_MARKERS)


def run_once(command, log_path, append):
    mode = "a" if append else "w"
    with log_path.open(mode, encoding="utf-8") as log:
        process = subprocess.Popen(
            command,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            bufsize=1,
        )
        assert process.stdout is not None
        for line in process.stdout:
            sys.stdout.write(line)
            sys.stdout.flush()
            log.write(line)
        return process.wait()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--log-file", type=Path, required=True)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if args.command[:1] == ["--"]:
        args.command = args.command[1:]
    if not args.command:
        parser.error("provide a command after --")

    args.log_file.parent.mkdir(parents=True, exist_ok=True)
    try:
        code = run_once(args.command, args.log_file, append=False)
    except OSError as error:
        print(f"retry-incremental: unable to run command: {error}", file=sys.stderr)
        return 127
    if code == 0:
        return 0

    output = args.log_file.read_text(encoding="utf-8", errors="replace")
    if not should_retry(output):
        return code

    print("retry-incremental: stale incremental failure detected; clearing caches and retrying")
    with args.log_file.open("a", encoding="utf-8") as log:
        log.write("\n[retry-incremental] stale incremental failure; retrying\n")
    for directory in DEFAULT_INCREMENTAL_DIRS:
        shutil.rmtree(directory, ignore_errors=True)
    try:
        return run_once(args.command, args.log_file, append=True)
    except OSError as error:
        print(f"retry-incremental: retry could not start: {error}", file=sys.stderr)
        return 127


if __name__ == "__main__":
    sys.exit(main())
