#!/usr/bin/env python3
"""Cargo adapter for cargo-dist: retain workspace features, build the CLI only."""

from __future__ import annotations

import os
import sys


def build_args(args: list[str]) -> list[str]:
    if not args or args[0] != "build":
        return args
    # A package build changes dependency feature unification (notably HTTP/2,
    # JSON map order and TLS roots). Require the workspace dist build contract.
    if "--workspace" not in args or "--profile" not in args:
        raise ValueError("expected cargo-dist's workspace build with --profile dist")
    index = args.index("--profile")
    if index + 1 == len(args) or args[index + 1] != "dist":
        raise ValueError("expected cargo-dist's --profile dist")
    selectors = (
        "--package", "--exclude", "--manifest-path", "--all", "--bin", "--bins",
        "--lib", "--test", "--tests", "--example", "--examples", "--bench",
        "--benches", "--all-targets",
    )
    if any(arg.startswith("-p") and not arg.startswith("--")
           or arg.split("=", 1)[0] in selectors for arg in args):
        raise ValueError("cargo-dist target selection changed; review the release adapter")
    return [*args, "--bin", "nyxid", "--locked"]


def main() -> None:
    cargo = os.environ["NYXID_DIST_CARGO"]
    if os.path.realpath(cargo) == os.path.realpath(__file__):
        raise ValueError("NYXID_DIST_CARGO must name the real Cargo executable")
    args = build_args(sys.argv[1:])
    # Do not leak the adapter into build scripts or other nested Cargo commands.
    os.environ["CARGO"] = cargo
    os.execv(cargo, [cargo, *args])


if __name__ == "__main__":
    main()
