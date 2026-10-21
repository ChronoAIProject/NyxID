#!/usr/bin/env python3
"""Bind the sealed wizard's actual bytes to a final uncompressed RustEmbed binary.

Run the Node wizard seal verifier first to validate source provenance. This
check independently requires every sealed asset, including the seal itself,
to be present verbatim in the final binary after linking/stripping.
"""
import argparse
import hashlib
import json
import mmap
from pathlib import Path, PurePosixPath
import stat

ROOT = Path(__file__).resolve().parents[1]
MAX_BINARY = 1024 * 1024 * 1024


def digest(data):
    return hashlib.sha256(data).hexdigest()


def verify(binary, assets):
    metadata = binary.lstat()
    if not stat.S_ISREG(metadata.st_mode) or not 0 < metadata.st_size <= MAX_BINARY:
        raise ValueError("final binary must be a bounded regular file")
    seal_path = assets / "shipped-composition.json"
    seal_bytes = seal_path.read_bytes()
    seal = json.loads(seal_bytes)
    if seal.get("schema") != "nyxid.shipped-composition.v1" or seal.get("component") != "wizard":
        raise ValueError("missing wizard final-byte seal")
    expected = {"shipped-composition.json": {"sha256": digest(seal_bytes), "size": len(seal_bytes)}}
    for record in seal["artifacts"]:
        name = record["path"]
        parts = PurePosixPath(name).parts
        if not parts or name.startswith("/") or "\\" in name or any(part in (".", "..") for part in parts) or name in expected:
            raise ValueError("invalid or duplicate sealed asset path")
        expected[name] = record
    actual = set()
    for path in assets.rglob("*"):
        mode = path.lstat().st_mode
        if stat.S_ISDIR(mode):
            continue
        if not stat.S_ISREG(mode):
            raise ValueError("nonregular wizard asset")
        actual.add(path.relative_to(assets).as_posix())
    if actual != set(expected):
        raise ValueError("wizard asset set differs from final-byte seal")
    records = []
    with binary.open("rb") as source, mmap.mmap(source.fileno(), 0, access=mmap.ACCESS_READ) as image:
        binary_digest = digest(image)
        for name, record in sorted(expected.items()):
            data = (assets / name).read_bytes()
            if not data or len(data) != record["size"] or digest(data) != record["sha256"]:
                raise ValueError("wizard asset differs from final-byte seal")
            offset = image.find(data)
            if offset < 0:
                raise ValueError("sealed wizard asset absent from final binary: " + name)
            records.append({"path": name, "offset": offset, "size": len(data), "sha256": digest(data)})
    return {"schema": "nyxid.embedded-wizard.v1", "subject": {"name": binary.name,
            "sha256": binary_digest, "size": metadata.st_size}, "seal_sha256": digest(seal_bytes), "assets": records}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--assets", type=Path, default=ROOT / "cli/src/wizard/assets")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.output.write_text(json.dumps(verify(args.binary, args.assets), indent=2) + "\n")
    print("Final binary contains every sealed wizard asset")
