#!/usr/bin/env python3
"""Package the verified raw committed export through cargo-dist's extra-artifact hook.

The source archive is a deterministic gzip of the exporter's exact source.tar.
No Git archive, attributes, generated files, or working-tree files are consumed.
Python 3.8 compatible; the receipt is evidence, not another archive member.
"""
import argparse
import gzip
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import sys

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("release_source_export", Path(__file__).with_name("export-build-source.py"))
source = importlib.util.module_from_spec(spec)
spec.loader.exec_module(source)
OUTPUT = Path("target/release-source")


def inspect(archive, exported):
    if archive.is_symlink() or not archive.is_file() or not 0 < archive.stat().st_size <= 1024 * 1024 * 1024:
        raise ValueError("expected bounded regular release source archive")
    digest, size = hashlib.sha256(), 0
    with gzip.open(archive, "rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            size += len(block)
            if size > exported["archive"]["size"]:
                raise ValueError("release source expands beyond exact raw export")
            digest.update(block)
    if size != exported["archive"]["size"] or digest.hexdigest() != exported["archive"]["sha256"]:
        raise ValueError("release source differs from exact raw committed export")
    source.require_archive_tree(archive, exported["committed_files"])
    return {"name": "source.tar.gz", "sha256": source.digest(archive), "size": archive.stat().st_size}


def build(tree, commit, epoch):
    tree = tree.resolve()
    destination = tree.parent
    if tree.name != "source":
        raise ValueError("source packaging requires the verified export root")
    exported = source.verify(destination, commit, ["target"])
    if exported["source"]["epoch"] != epoch:
        raise ValueError("source packaging epoch differs from verified export")
    output = tree / OUTPUT
    # cargo-dist 0.30 can return zero after a failed nested build if outputs
    # already exist. Never reuse this hook's outputs, even for the same commit.
    if (tree / "target").is_symlink():
        raise ValueError("source output parent must be a regular directory")
    output.mkdir(parents=True, exist_ok=False)
    archive = output / "source.tar.gz"
    with archive.open("xb") as target:
        with gzip.GzipFile(filename="", mode="wb", fileobj=target, mtime=0) as compressed:
            with (destination / "source.tar").open("rb") as raw:
                shutil.copyfileobj(raw, compressed, length=1024 * 1024)
    subject = inspect(archive, exported)
    checksum = output / "source.tar.gz.sha256"
    with checksum.open("x") as stream:
        stream.write(subject["sha256"] + " *source.tar.gz\n")
    source.verify(destination, commit, ["target"])
    receipt = {"schema": "nyxid.release-source.v1", "source": exported["source"],
               "raw_archive": exported["archive"], "subject": subject,
               "export_receipt_sha256": source.digest(destination / "source-export.json"),
               "checksum": {"name": checksum.name, "sha256": source.digest(checksum)},
               "post_build_source_verified": True}
    with (output / "source-archive.json").open("x") as stream:
        json.dump(receipt, stream, indent=2)
        stream.write("\n")
    return receipt


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.parse_args()
    commit = os.environ.get("NYXID_GIT_HASH", "")
    epoch = os.environ.get("SOURCE_DATE_EPOCH", "")
    if not epoch.isdecimal():
        parser.error("verified full source commit and epoch are required")
    print(json.dumps(build(Path.cwd(), commit, int(epoch))))


if __name__ == "__main__":
    main()
