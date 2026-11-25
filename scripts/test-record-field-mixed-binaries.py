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
    parser.add_argument("--shared-signing", action="store_true",
                        help="prepare shared signing producers and verify reader-capable rollback")
    parser.add_argument("--base-reads-local-pins", action="store_true",
                        help="the base already implements bot-local continuity")
    parser.add_argument("--channel-writers", action="store_true",
                        help="exercise active and rolled-back channel writers against the dual-reader base")
    parser.add_argument("--whatsapp", action="store_true",
                        help="exercise retained enrollment migration and token replacement after writer rollback")
    parser.add_argument("--gateway-key", action="store_true",
                        help="exercise managed channel-key replacement, removal and retained migration")
    parser.add_argument("--event-bindings", action="store_true",
                        help="check metadata-only serving and storage verification for two retained credentials")
    parser.add_argument("--gateway-setup", action="store_true",
                        help="replay a retained setup at its loopback gateway on both server revisions")
    parser.add_argument("--cma-requests", action="store_true",
                        help="exchange both request copies across two dual-reader revisions")
    parser.add_argument("--learning-writers", action="store_true",
                        help="exchange learning proposals with the reader-preparation binary")
    parser.add_argument("--approval-readers", action="store_true",
                        help="exchange four versions of pending, completed and cleared approvals")
    parser.add_argument("--approval-writers", action="store_true",
                        help="migrate and exchange retained approvals with the dual-reader base")
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
        def run(side, step, fixture, *, writes=False, legacy=True, expect_pin_mismatch=False, test=args.test):
            print(f"{side} {step} {fixture} (contextual writes {writes}, legacy reads {legacy})", flush=True)
            env = dict(os.environ, NYXID_RECORD_FIXTURE_STEP=step,
                       NYXID_RECORD_FIXTURE_FILE=str(Path(scratch) / fixture),
                       CONTEXTUAL_RECORD_WRITES=str(writes).lower(),
                       CONTEXTUAL_RECORD_WRITERS_DRAINED=str(writes).lower(),
                       ALLOW_LEGACY_RECORD_READS=str(legacy).lower())
            command = [bins[side], "--exact", test]
            if expect_pin_mismatch:
                result = subprocess.run(command, env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
                if result.returncode != 101 or not all(s in result.stdout for s in ("assertion", "mixed_records.rs", "verification")):
                    raise RuntimeError("base reader did not report the expected stored fingerprint mismatch")
                print("base opened the fixture and rejected its retained fingerprint", flush=True)
            else:
                result = subprocess.run(command, env=env, text=True, stdout=subprocess.PIPE,
                                        stderr=subprocess.STDOUT)
                print(result.stdout, flush=True)
                result.check_returncode()
                if "1 passed;" not in result.stdout:
                    raise RuntimeError(f"test did not run: {test}")

        if args.gateway_setup:
            for writer, reader in [("base", "candidate"), ("candidate", "base")]:
                run(writer, "write", writer)
                run(reader, "read", writer)
                run(reader, "replace", writer)
                run(writer, "read", writer)
                run(writer, "retire", writer)
            run("base", "write", "old")
            run("candidate", "read", "old", writes=True)
            run("candidate", "reject", "old", writes=True, legacy=False)
            run("candidate", "migrate", "old", writes=True)
            run("candidate", "read", "old", legacy=False)
            run("base", "reject", "old")
            run("candidate", "write", "current", writes=True)
            run("candidate", "read", "current", writes=True, legacy=False)
            run("base", "reject", "current")
            run("candidate", "read", "current")
            run("candidate", "replace", "current")
            run("base", "read", "current")
            registry = "services::encrypted_record_migration::mixed_webhooks::mixed_binary_webhook_signatures"
            for writer, reader, previous in [("base", "candidate", "known"), ("candidate", "base", "unknown")]:
                run(writer, "registry-write", "generation", writes=True, test=registry)
                run(writer, "registry-live", "generation", writes=True, test=registry)
                run(reader, "registry-" + previous, "generation", writes=True, test=registry)
            return

        if args.approval_writers:
            for writer, reader in [("base", "candidate"), ("candidate", "base")]:
                run(writer, "write", writer)
                run(reader, "read", writer)
            run("base", "write", "base-on", writes=True)
            run("candidate", "read", "base-on", writes=True)
            run("candidate", "reject", "base-on", writes=True, legacy=False)
            run("candidate", "migrate", "base-on", writes=True)
            run("candidate", "write", "active", writes=True, legacy=False)
            for fixture in ["base-on", "active"]:
                for side in ["base", "candidate"]:
                    run(side, "read", fixture, writes=True, legacy=False)
                    run(side, "read", fixture, legacy=False)
            run("candidate", "write", "rollback")
            run("base", "read", "rollback")
            run("candidate", "reject", "rollback", legacy=False)
            registry = "services::encrypted_record_migration::mixed_webhooks::mixed_binary_webhook_signatures"
            for writer, reader, previous in [("base", "candidate", "known"), ("candidate", "base", "unknown")]:
                run(writer, "registry-write", "generation", writes=True, test=registry)
                run(writer, "registry-live", "generation", writes=True, test=registry)
                run(reader, "registry-" + previous, "generation", test=registry, writes=True)
            return

        if args.approval_readers:
            for writes in (False, True):
                for writer, reader in [("base", "candidate"), ("candidate", "base")]:
                    run(writer, "write", "legacy", writes=writes)
                    run(reader, "read", "legacy", writes=writes)
            run("candidate", "reject", "legacy", legacy=False)
            run("candidate", "contextual-fixture", "current", writes=True)
            run("candidate", "read", "current", writes=True, legacy=False)
            run("candidate", "read", "current", legacy=False)
            run("base", "reject", "current")
            return

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
        if args.learning_writers:
            run("base", "write", "base-on", writes=True)
            run("candidate", "read", "base-on", writes=True)
            run("candidate", "reject", "old", writes=True, legacy=False)
            run("candidate", "migrate", "old", writes=True)
            run("base", "read", "old", legacy=False)
            run("candidate", "read", "old", legacy=False)
            run("candidate", "write", "current", writes=True)
            for side in ["base", "candidate"]:
                run(side, "read", "current", writes=True, legacy=False)
                run(side, "read", "current", legacy=False)
            run("candidate", "replace", "current", writes=True)
            run("base", "read", "current", legacy=False)
            run("base", "replace", "current")
            run("candidate", "read", "current")
            run("candidate", "replace", "current", writes=True)
            run("candidate", "clear", "current")
            run("base", "read", "current", legacy=False)
            registry = "services::encrypted_record_migration::mixed_webhooks::mixed_binary_webhook_signatures"
            for writer, reader, previous in [("base", "candidate", "known"), ("candidate", "base", "unknown")]:
                run(writer, "registry-write", "generation", writes=True, test=registry)
                run(writer, "registry-live", "generation", writes=True, test=registry)
                run(reader, "registry-" + previous, "generation", writes=True, test=registry)
            return
        if args.cma_requests:
            for writer, reader in [("base", "candidate"), ("candidate", "base")]:
                run(writer, "write", "active", writes=True)
                run(reader, "read", "active", legacy=False)
                run(reader, "replace", "active", writes=True)
                run(writer, "read", "active", legacy=False)
                run(reader, "replace", "active")
                run(writer, "read", "active")
                run(writer, "clear", "active")
                run(reader, "read", "active", legacy=False)
            run("candidate", "reject", "old", writes=True, legacy=False)
            run("candidate", "migrate", "old", writes=True)
            run("base", "read", "old", legacy=False)
            run("candidate", "read", "old", legacy=False)
            registry = "services::encrypted_record_migration::mixed_webhooks::mixed_binary_webhook_signatures"
            for writer, reader, previous in [("base", "candidate", "known"), ("candidate", "base", "unknown")]:
                run(writer, "registry-write", "generation", writes=True, test=registry)
                run(writer, "registry-live", "generation", writes=True, test=registry)
                run(reader, "registry-" + previous, "generation", writes=True, test=registry)
            return
        if args.channel_writers:
            run("candidate", "rewrap-pinned", "old", writes=True)
            run("base", "read-pinned-strict", "old", legacy=False)
            run("candidate", "read-pinned-strict", "old", writes=True, legacy=False)
            run("base", "write", "old")
        if args.pin_continuity or args.shared_signing:
            run("base", "write", "pinned")
            run("candidate", "rewrap-pinned", "pinned", writes=True)
            run("candidate", "read", "pinned", writes=True)
            run("candidate", "read-pinned-strict", "pinned", writes=True, legacy=False)
            if args.shared_signing:
                run("base", "reject-shared", "pinned")
            else:
                run("base", "read", "pinned", expect_pin_mismatch=not args.base_reads_local_pins)
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
        if args.event_bindings:
            run("candidate", "verify", "current", writes=True, legacy=False)
            run("base", "metadata", "current")
        else:
            run("base", "read" if args.channel_writers else "reject", "current")
        run("candidate", "read", "current")
        if args.reader_preparation:
            return
        if args.whatsapp or args.gateway_key or args.event_bindings:
            run("candidate", "migrate", "old", writes=True)
            run("candidate", "read", "old", legacy=False)
            run("base", "metadata" if args.event_bindings else "reject", "old")
            run("candidate", "replace", "current")
            run("base", "read", "current")
            run("candidate", "replace", "current", writes=True)
            run("candidate", "clear", "current")
            run("base", "read", "current")
        registry_test = args.test
        if args.whatsapp or args.gateway_key or args.event_bindings:
            registry_test = "services::encrypted_record_migration::mixed_webhooks::mixed_binary_webhook_signatures"
        if args.channel_writers:
            run("candidate", "replace", "current", writes=True)
            run("base", "read", "current", legacy=False)
            run("candidate", "clear", "current", writes=True)
            run("base", "read", "current", legacy=False)
            run("candidate", "replace", "current")
            run("base", "read", "current")
            run("candidate", "replace", "current", writes=True)
            run("base", "replace", "current")
            run("candidate", "read", "current")
            registry_test = "services::encrypted_record_migration::mixed_webhooks::mixed_binary_webhook_signatures"
        for writer, reader, previous in [("base", "candidate", "known"), ("candidate", "base", "unknown")]:
            run(writer, "registry-write", "generation", writes=True, test=registry_test)
            run(writer, "registry-live", "generation", writes=True, test=registry_test)
            run(reader, "registry-" + previous, "generation", writes=True, test=registry_test)


if __name__ == "__main__":
    main()
