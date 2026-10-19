#!/usr/bin/env python3
"""Produce review-only APK candidates; never used by a product image build.

The exact target base's APK state/keyring is copied into an isolated solver
root. --no-scripts permits cross-architecture resolution without executing
foreign binaries. Native PR closure replay must pass before accepting a pin.
"""
import argparse
from pathlib import Path
import platform
import subprocess

from apt_inputs import stages

ROOT = Path(__file__).resolve().parents[1]
from resolve_apk_requests import REQUESTS


def base_image(label):
    name, stage, _ = REQUESTS[label]
    parsed = stages((ROOT / name).read_text())
    return next(s.base for s in parsed if s.name == stage)


def run(command, **kwargs):
    return subprocess.run(command, check=True, **kwargs)


def generate(args):
    native = {"x86_64": "amd64", "aarch64": "arm64", "arm64": "arm64"}[platform.machine()]
    for label in args.stage:
        base = base_image(label)
        output = args.output.resolve() / (label + "-" + args.arch)
        output.mkdir(parents=True, exist_ok=False)
        target = subprocess.check_output(["docker", "create", "--platform", "linux/" + args.arch,
                                          "--entrypoint", "sh", base, "-c", "true"], text=True).strip()
        try:
            for path in ("lib/apk", "etc/apk"):
                dest = output / "solver" / path
                dest.parent.mkdir(parents=True, exist_ok=True)
                run(["docker", "cp", target + ":/" + path, str(dest)])
        finally:
            run(["docker", "rm", target], stdout=subprocess.DEVNULL)
        (output / "solver/var/cache/apk").mkdir(parents=True)
        # Rootfs belongs to the candidate only; no application image is built.
        run(["docker", "run", "--rm", "--user", "0", "--platform", "linux/" + native, "--memory=1536m", "--cpus=2",
             "--read-only", "--tmpfs", "/tmp", "--entrypoint", "sh",
             "-v", str(ROOT / "scripts") + ":/input:ro", "-v", str(output) + ":/out",
             "-e", "NYXID_APK_RESOLVE_ROOT=/out/solver",
             "-e", "NYXID_APK_RESOLVE_ARCH=" + {"amd64": "x86_64", "arm64": "aarch64"}[args.arch],
             base, "/input/resolve-apk-inputs.sh", base, *REQUESTS[label][2]])
        print("CANDIDATE " + str(output / "archives.tsv"), flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--arch", choices=("amd64", "arm64"), required=True)
    parser.add_argument("--stage", choices=tuple(REQUESTS), action="append")
    parser.add_argument("--output", type=Path, required=True)
    arguments = parser.parse_args()
    arguments.stage = arguments.stage or list(REQUESTS)
    generate(arguments)
