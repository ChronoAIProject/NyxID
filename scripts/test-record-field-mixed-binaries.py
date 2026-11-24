#!/usr/bin/env python3
"""Exercise a record fixture compiled into two server test binaries on test MongoDB."""
import argparse
import hashlib
import os
from pathlib import Path
import subprocess
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("base", type=Path)
    parser.add_argument("candidate", type=Path)
    parser.add_argument("test", help="fully qualified Rust test name")
    parser.add_argument("--mutations", nargs="*", default=[], help="family-specific rejection fixtures")
    parser.add_argument("--reader-preparation", action="store_true",
                        help="check unchanged writers in both modes and a test-built contextual row")
    parser.add_argument("--pin-continuity", action="store_true",
                        help="rewrap a base writer's pinned credentials and read them after writer rollback")
    args = parser.parse_args()
    bins = {"base": args.base.resolve(), "candidate": args.candidate.resolve()}
    hashes = {}
    for side, binary in bins.items():
        digest = hashlib.sha256()
        with binary.open("rb") as source:
            for block in iter(lambda: source.read(1024 * 1024), b""):
                digest.update(block)
        hashes[side] = digest.hexdigest()
        print(f"{side} binary SHA-256 {hashes[side]}", flush=True)
        tests = subprocess.check_output([binary, "--list", "--exact", args.test], text=True)
        if f"{args.test}: test" not in tests.splitlines():
            parser.error(f"{binary} does not include {args.test}")
    if hashes["candidate"] == hashes["base"]:
        parser.error("base and candidate must be independently built versions")
    with tempfile.TemporaryDirectory(prefix="nyxid-record-fixture-") as scratch:
        def run(side, step, fixture, *, writes=False, legacy=True, expect_pin_mismatch=False):
            print(f"{side} {step} {fixture} (contextual writes {writes}, legacy reads {legacy})", flush=True)
            env = dict(os.environ, NYXID_RECORD_FIXTURE_STEP=step,
                       NYXID_RECORD_FIXTURE_FILE=str(Path(scratch) / fixture),
                       CONTEXTUAL_RECORD_WRITES=str(writes).lower(),
                       CONTEXTUAL_RECORD_WRITERS_DRAINED=str(writes).lower(),
                       ALLOW_LEGACY_RECORD_READS=str(legacy).lower())
            command = [bins[side], "--exact", args.test]
            if expect_pin_mismatch:
                result = subprocess.run(command, env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
                if result.returncode != 101 or not all(s in result.stdout for s in ("assertion", "mixed_records.rs", "verification")):
                    raise RuntimeError("base reader did not report the expected stored fingerprint mismatch")
                print("base opened the fixture and rejected its retained fingerprint", flush=True)
            else:
                subprocess.run(command, env=env, check=True)

        run("base", "write", "old")
        run("candidate", "read", "old")
        run("candidate", "write", "compatible")
        run("base", "read", "compatible")
        for writer, reader, fixture in [("base", "candidate", "compatible"), ("candidate", "base", "old")]:
            run(writer, "replace", fixture)
            run(reader, "read", fixture)
            run(writer, "clear", fixture)
            run(reader, "read", fixture)
        # Keep a populated legacy row for strict-mode and transition checks.
        run("base", "write", "old")
        if args.pin_continuity:
            run("base", "write", "pinned")
            run("candidate", "rewrap-pinned", "pinned", writes=True)
            run("candidate", "read", "pinned", writes=True)
            run("candidate", "read-pinned-strict", "pinned", writes=True, legacy=False)
            run("base", "read", "pinned", expect_pin_mismatch=True)
            run("candidate", "read", "pinned")
            run("candidate", "replace", "pinned")
            run("candidate", "read", "pinned")
            run("candidate", "clear", "pinned")
            run("candidate", "read", "pinned")
            return
        if args.reader_preparation:
            for writer, reader in [("base", "candidate"), ("candidate", "base")]:
                run(writer, "write", "writer-on", writes=True)
                run(reader, "read", "writer-on")
                run(writer, "replace", "writer-on", writes=True)
                run(reader, "read", "writer-on")
                run(writer, "clear", "writer-on", writes=True)
                run(reader, "read", "writer-on")
            run("candidate", "contextual-fixture", "current", writes=True)
        else:
            run("candidate", "write", "current", writes=True)
        run("candidate", "read", "old", writes=True)
        run("candidate", "read", "current", writes=True, legacy=False)
        run("candidate", "reject", "old", writes=True, legacy=False)
        for mutation in args.mutations:
            run("candidate", mutation, "current", writes=True)
        run("base", "reject", "current")
        run("candidate", "read", "current")
        if args.reader_preparation:
            return
        for writer, reader, previous in [("base", "candidate", "known"), ("candidate", "base", "unknown")]:
            run(writer, "registry-write", "generation", writes=True)
            run(writer, "registry-live", "generation", writes=True)
            run(reader, "registry-" + previous, "generation", writes=True)


if __name__ == "__main__":
    main()
