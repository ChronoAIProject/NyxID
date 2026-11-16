#!/usr/bin/env python3
"""Retry a compiler command once without incremental state after compiler failure.

The retry is deliberately narrow: ordinary compiler errors and test failures
are returned unchanged. The command's combined output is streamed to stdout
and copied to a log so the marker check works without hiding CI diagnostics.
"""

import argparse
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys

import incremental_cache


RETRY_MARKERS = (
    "internal compiler error",
    "incremental compilation",
    "failed to load dep graph",
    "dep-graph",
    "out of memory",
    "memory allocation of",
    "cannot allocate memory",
    "signal: 9, sigkill",
    "signal: 15, sigterm",
)


def should_retry(output, code=1):
    """Recognize stale state, allocation failures and killed compiler children."""
    lowered = output.lower()
    return code in (137, 143, -signal.SIGKILL, -signal.SIGTERM) or any(
        marker in lowered for marker in RETRY_MARKERS)


def run_once(command, log_path, append, env):
    mode = "a" if append else "w"
    with log_path.open(mode, encoding="utf-8") as log:
        process = subprocess.Popen(
            command,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            bufsize=1,
            env=env,
        )
        assert process.stdout is not None
        for line in process.stdout:
            sys.stdout.write(line)
            sys.stdout.flush()
            log.write(line)
        code = process.wait()
        incremental_cache.report_inventory('after-build')
        return 128 - code if code < 0 else code


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
    env = os.environ.copy()
    try:
        code = run_once(args.command, args.log_file, append=False, env=env)
    except OSError as error:
        print(f"retry-incremental: unable to run command: {error}", file=sys.stderr)
        return 127
    if code == 0:
        return 0

    output = args.log_file.read_text(encoding="utf-8", errors="replace")
    if not should_retry(output, code):
        return code

    print("retry-incremental: compiler state/memory failure; clearing graphs and retrying with CARGO_INCREMENTAL=0")
    with args.log_file.open("a", encoding="utf-8") as log:
        log.write("\n[retry-incremental] state/memory failure; retrying with CARGO_INCREMENTAL=0\n")
    for directory in incremental_cache.ROOTS:
        shutil.rmtree(directory, ignore_errors=True)
    env['CARGO_INCREMENTAL'] = '0'
    # Later plain-Cargo test steps must reuse this successful fallback build,
    # rather than switching back to incremental and compiling it all again.
    if github_env := os.environ.get('GITHUB_ENV'):
        with open(github_env, 'a', encoding='utf-8') as output:
            output.write('CARGO_INCREMENTAL=0\n')
    try:
        return run_once(args.command, args.log_file, append=True, env=env)
    except OSError as error:
        print(f"retry-incremental: retry could not start: {error}", file=sys.stderr)
        return 127


if __name__ == "__main__":
    sys.exit(main())
