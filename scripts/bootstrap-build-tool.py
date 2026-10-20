#!/usr/bin/env python3
"""Install one reviewed build tool; no downloaded installer shell is executed.

The committed catalog is the integrity authority. Refresh its version/digest
through review, never by fetching a checksum alongside executable bytes at boot.
Works with Python 3.8+ in the pinned focal release container.
"""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import platform
import shutil
import shlex
import subprocess
import tarfile
import tempfile
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
MAX_DOWNLOAD = 128 * 1024 * 1024
MAX_EXECUTABLE = 256 * 1024 * 1024


def target_for_host(machine, system, libc=None):
    arch = {"arm64": "aarch64", "aarch64": "aarch64", "amd64": "x86_64", "x86_64": "x86_64"}.get(machine)
    system = "apple-darwin" if system == "Darwin" else "unknown-linux-" + libc if system == "Linux" and libc in ("gnu", "musl") else None
    if not arch or not system:
        raise ValueError("unsupported build host or libc")
    return arch + "-" + system


def host_target():
    libc = None
    if platform.system() == "Linux":
        try:
            if os.confstr("CS_GNU_LIBC_VERSION").startswith("glibc "):
                libc = "gnu"
        except (ValueError, OSError, AttributeError):
            pass
        # Python's libc_ver often returns empty on Alpine. The runtime loader
        # is authoritative there; do not default unknown Linux to GNU.
        if libc is None and platform.libc_ver()[0] == "musl":
            libc = "musl"
        if libc is None and any(Path("/lib").glob("ld-musl-*.so.1")):
            libc = "musl"
    return target_for_host(platform.machine(), platform.system(), libc)


def target_input(tool, item, target):
    # cargo-auditable is a host tool, independent of the Rust build target.
    # Its pinned GNU binaries require newer glibc than the reviewed Focal
    # container; use the same-architecture pinned static MUSL host binary.
    if tool == "cargo-auditable" and target in ("aarch64-unknown-linux-gnu", "x86_64-unknown-linux-gnu"):
        target = target[:-3] + "musl"
    if "targets" not in item:
        return item
    if target not in item["targets"]:
        raise ValueError("unsupported " + tool + " host target: " + target)
    return item["targets"][target]


def download_verified(item, path):
    algorithm, expected = ("sha256", item.get("sha256"))
    if expected is None:
        algorithm, encoded = item["integrity"].split("-", 1)
        if algorithm != "sha512":
            raise ValueError("unsupported tool integrity algorithm")
        expected = base64.b64decode(encoded, validate=True).hex()
    digest = hashlib.new(algorithm)
    size = 0
    if not item["url"].startswith("https://"):
        raise ValueError("tool URL must use HTTPS")
    with urllib.request.urlopen(item["url"], timeout=60) as response, path.open("wb") as output:
        if not response.url.startswith("https://"):
            raise ValueError("tool redirect must use HTTPS")
        while True:
            chunk = response.read(64 * 1024)
            if not chunk:
                break
            size += len(chunk)
            if size > MAX_DOWNLOAD:
                raise ValueError("tool archive exceeds bound")
            digest.update(chunk)
            output.write(chunk)
    if digest.hexdigest() != expected:
        raise ValueError("build tool checksum mismatch")


def unpack_npm(archive, destination):
    total = 0
    with tarfile.open(archive) as package:
        for member in package:
            parts = PurePosixPath(member.name).parts
            if not parts or parts[0] != "package" or ".." in parts or "\\" in member.name:
                raise ValueError("invalid npm archive path")
            if member.isdir():
                continue
            if not member.isfile():
                raise ValueError("nonregular npm archive member")
            total += member.size
            if total > MAX_DOWNLOAD:
                raise ValueError("expanded npm archive exceeds bound")
            target = destination.joinpath(*parts[1:])
            target.parent.mkdir(parents=True, exist_ok=True)
            with package.extractfile(member) as source, target.open("wb") as output:
                shutil.copyfileobj(source, output)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("tool", choices=["cargo-dist", "cargo-auditable", "syft", "gitleaks", "trivy", "cargo-audit", "actionlint", "gh", "regctl", "rustup", "npm"])
    parser.add_argument("--bin-dir", type=Path, required=True)
    args = parser.parse_args()
    catalog = json.loads((ROOT / "security/build-tools.json").read_text())
    args.bin_dir.mkdir(parents=True, exist_ok=True)
    destination = args.bin_dir.resolve()
    target = host_target()
    item = target_input(args.tool, catalog[args.tool], target)
    with tempfile.TemporaryDirectory(prefix="nyxid-build-tool-", dir=destination) as temporary:
        stage = Path(temporary)
        # rustup dispatches by argv[0]. Verified installer bytes must retain
        # the installer basename instead of entering an unsupported proxy mode.
        archive = stage / ("rustup-init" if args.tool == "rustup" else "download")
        download_verified(item, archive)
        if args.tool in ("cargo-dist", "cargo-auditable", "syft", "gitleaks", "trivy", "cargo-audit", "actionlint", "gh"):
            executable = "dist" if args.tool == "cargo-dist" else args.tool
            names = ("dist", "cargo-dist") if args.tool == "cargo-dist" else (executable,)
            with tarfile.open(archive) as package:
                members = [m for m in package if m.isfile() and PurePosixPath(m.name).name in names]
                if len(members) != 1 or members[0].size > MAX_EXECUTABLE:
                    raise ValueError("expected one reviewed tool executable")
                binary = stage / executable
                with package.extractfile(members[0]) as source, binary.open("wb") as output:
                    shutil.copyfileobj(source, output)
                binary.chmod(0o755)
                os.replace(binary, destination / executable)
            version_args = ["auditable", "--version"] if args.tool == "cargo-auditable" else ["version"] if args.tool in ("syft", "gitleaks") else ["--version"]
            subprocess.run([str(destination / executable), *version_args], check=True)
        elif args.tool == "regctl":
            archive.chmod(0o755)
            os.replace(archive, destination / "regctl")
            subprocess.run([str(destination / "regctl"), "version"], check=True)
        elif args.tool == "rustup":
            archive.chmod(0o755)
            subprocess.run([str(archive), "-y", "--no-modify-path", "--profile", "minimal", "--default-toolchain", catalog["rust_toolchain"]], check=True)
        else:
            package = stage / "npm"
            package.mkdir()
            unpack_npm(archive, package)
            if json.loads((package / "package.json").read_text())["version"] != catalog["npm"]["version"]:
                raise ValueError("npm version mismatch")
            installed = destination / ("npm-" + catalog["npm"]["version"])
            if installed.exists():
                raise ValueError("npm destination already exists; use a clean build tool directory")
            os.replace(package, installed)
            for command in ("npm", "npx"):
                wrapper = destination / command
                wrapper.write_text('#!/bin/sh\nexec node ' + shlex.quote(str(installed / "bin" / (command + "-cli.js"))) + ' "$@"\n')
                wrapper.chmod(0o755)
            subprocess.run([str(destination / "npm"), "--version"], check=True)
    print("Verified " + args.tool + " " + catalog[args.tool]["version"])


if __name__ == "__main__":
    main()
