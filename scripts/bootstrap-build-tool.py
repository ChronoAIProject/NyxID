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


def host_target():
    arch = {"arm64": "aarch64", "aarch64": "aarch64", "amd64": "x86_64", "x86_64": "x86_64"}.get(platform.machine())
    system = {"Darwin": "apple-darwin", "Linux": "unknown-linux-gnu"}.get(platform.system())
    if not arch or not system:
        raise ValueError("unsupported build host")
    return arch + "-" + system


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
    parser.add_argument("tool", choices=["cargo-dist", "rustup", "npm"])
    parser.add_argument("--bin-dir", type=Path, required=True)
    args = parser.parse_args()
    catalog = json.loads((ROOT / "security/build-tools.json").read_text())
    args.bin_dir.mkdir(parents=True, exist_ok=True)
    destination = args.bin_dir.resolve()
    target = host_target()
    item = catalog[args.tool]
    if "targets" in item:
        item = item["targets"][target]
    with tempfile.TemporaryDirectory(prefix="nyxid-build-tool-", dir=destination) as temporary:
        stage = Path(temporary)
        archive = stage / "download"
        download_verified(item, archive)
        if args.tool == "cargo-dist":
            with tarfile.open(archive) as package:
                members = [m for m in package if m.isfile() and PurePosixPath(m.name).name in ("dist", "cargo-dist")]
                if len(members) != 1 or members[0].size > MAX_DOWNLOAD:
                    raise ValueError("expected one dist executable")
                binary = stage / "dist"
                with package.extractfile(members[0]) as source, binary.open("wb") as output:
                    shutil.copyfileobj(source, output)
                binary.chmod(0o755)
                os.replace(binary, destination / "dist")
            subprocess.run([str(destination / "dist"), "--version"], check=True)
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
