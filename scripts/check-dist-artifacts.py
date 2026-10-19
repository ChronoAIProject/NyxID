#!/usr/bin/env python3
"""Inspect planned CLI archives without executing their foreign binaries.

Coverage comes from the validated plan, never an artifact glob. This bounded
inspection writes binary and archive SBOMs into a separate evidence directory.
"""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path, PurePosixPath
import struct
import subprocess
import tarfile
import tempfile
import uuid

from publication_contract import file_digest, load_json, validate_plan

spec = importlib.util.spec_from_file_location("rust_binary_sbom", Path(__file__).with_name("rust-binary-sbom.py"))
binary_sbom = importlib.util.module_from_spec(spec)
spec.loader.exec_module(binary_sbom)
MAX_ARCHIVE = 512 * 1024 * 1024
MAX_EXPANDED = 1024 * 1024 * 1024


def check_binary_target(binary, target):
    with binary.open("rb") as source:
        header = source.read(64)
    arch, platform = target.split("-", 1)
    if platform == "apple-darwin":
        cpu = {"aarch64": 0x0100000C, "x86_64": 0x01000007}.get(arch)
        if len(header) < 32 or header[:4] != b"\xcf\xfa\xed\xfe" or struct.unpack_from("<I", header, 4)[0] != cpu or struct.unpack_from("<I", header, 12)[0] != 2:
            raise ValueError("archive binary architecture/format differs from planned Darwin target")
    elif platform == "unknown-linux-gnu":
        machine = {"aarch64": 183, "x86_64": 62}.get(arch)
        if len(header) < 64 or header[:6] != b"\x7fELF\x02\x01" or struct.unpack_from("<H", header, 18)[0] != machine:
            raise ValueError("archive binary architecture/format differs from planned Linux target")
        if struct.unpack_from("<H", header, 16)[0] not in (2, 3) or not struct.unpack_from("<Q", header, 24)[0]:
            raise ValueError("archive ELF is not an executable")
        offset = struct.unpack_from("<Q", header, 32)[0]
        size, count = struct.unpack_from("<HH", header, 54)
        if size != 56 or not 1 <= count <= 128 or offset + size * count > min(binary.stat().st_size, 1024 * 1024):
            raise ValueError("invalid/bounded ELF program headers required")
        interpreters = []
        with binary.open("rb") as source:
            source.seek(offset)
            programs = source.read(size * count)
            for index in range(count):
                fields = struct.unpack_from("<IIQQQQQQ", programs, index * size)
                if fields[0] != 3:
                    continue
                position, length = fields[2], fields[5]
                if not 1 < length <= 256 or position + length > binary.stat().st_size:
                    raise ValueError("invalid ELF interpreter")
                source.seek(position)
                interpreters.append(source.read(length))
        expected = {"aarch64": b"/lib/ld-linux-aarch64.so.1\x00", "x86_64": b"/lib64/ld-linux-x86-64.so.2\x00"}[arch]
        if interpreters != [expected]:
            raise ValueError("archive ELF interpreter differs from planned GNU ABI")
    else:
        raise ValueError("unsupported planned binary target")


def inspect_archive(path, subject, destination):
    digest = file_digest(path, MAX_ARCHIVE)
    expected = {item["path"] for item in subject["assets"]}
    prefix = subject["name"][:-len(".tar.gz")]
    found, total = {}, 0
    layout = None
    with tarfile.open(path, mode="r:gz") as archive:
        for index, member in enumerate(archive):
            if index >= 64:
                raise ValueError("archive member count exceeds bound")
            parts = PurePosixPath(member.name).parts
            if not parts or member.name.startswith("/") or "\\" in member.name or any(part in ("", ".", "..") for part in member.name.rstrip("/").split("/")):
                raise ValueError("unsafe archive member path")
            if member.isdir() and member.name.rstrip("/") == prefix:
                continue
            member_layout = "prefixed" if len(parts) == 2 and parts[0] == prefix else "flat" if len(parts) == 1 else None
            if member_layout is None or (layout is not None and layout != member_layout):
                raise ValueError("unexpected archive directory layout")
            layout = member_layout
            name = parts[-1]
            if name not in expected or name in found or not member.isfile() or not 0 < member.size <= MAX_EXPANDED:
                raise ValueError("archive has missing/duplicate/extra/nonregular planned assets")
            total += member.size
            if total > MAX_EXPANDED:
                raise ValueError("expanded archive exceeds bound")
            hasher = hashlib.sha256()
            written = 0
            output = (destination / name).open("wb") if name == subject["binary"] else None
            try:
                with archive.extractfile(member) as source:
                    for block in iter(lambda: source.read(65536), b""):
                        written += len(block)
                        hasher.update(block)
                        if output:
                            output.write(block)
            finally:
                if output:
                    output.close()
            if written != member.size:
                raise ValueError("truncated archive member")
            found[name] = {"path": name, "sha256": hasher.hexdigest(), "size": written}
    if set(found) != expected:
        raise ValueError("archive is missing planned assets")
    binary = destination / subject["binary"]
    check_binary_target(binary, subject["target"])
    return {"digest": digest, "size": path.stat().st_size, "assets": [found[name] for name in sorted(found)]}


def archive_sbom(binary_document, subject, inspection):
    document = json.loads(json.dumps(binary_document))
    binary_component = document["metadata"]["component"]
    binary_ref = binary_component["bom-ref"]
    archive_ref = inspection["digest"]
    component = {"type": "file", "name": subject["name"], "bom-ref": archive_ref,
                 "hashes": [{"alg": "SHA-256", "content": archive_ref.split(":")[1]}],
                 "properties": binary_component["properties"] + [{"name": "nyxid:target", "value": subject["target"]}]}
    document["metadata"]["component"] = component
    document["serialNumber"] = "urn:uuid:" + str(uuid.uuid5(uuid.NAMESPACE_URL, archive_ref))
    document["components"].append(binary_component)
    member_refs = []
    for asset in inspection["assets"]:
        if asset["path"] == subject["binary"]:
            continue
        ref = archive_ref + ":" + asset["path"]
        member_refs.append(ref)
        document["components"].append({"type": "file", "bom-ref": ref, "name": asset["path"],
                                       "hashes": [{"alg": "SHA-256", "content": asset["sha256"]}]})
    document["dependencies"].append({"ref": archive_ref, "dependsOn": [binary_ref, *member_refs]})
    return document


def inspect(plan, artifacts, evidence, extractor, syft, targets=None, source_diff=None):
    validate_plan(plan)
    subjects = [item for item in plan["subjects"] if item.get("artifact_kind") == "executable-zip"]
    expected_targets = {item["target"] for item in subjects}
    if targets is not None:
        if not targets or not set(targets) <= expected_targets or len(targets) != len(set(targets)):
            raise ValueError("local archive targets differ from validated plan")
        subjects = [item for item in subjects if item["target"] in targets]
    evidence.mkdir(parents=True, exist_ok=False)
    records = []
    for subject in subjects:
        stem = subject["name"]
        with tempfile.TemporaryDirectory(prefix="nyxid-inspect-") as temporary:
            directory = Path(temporary)
            inspection = inspect_archive(artifacts / stem, subject, directory)
            binary = directory / subject["binary"]
            receipt_path = evidence / (stem + ".binary-info.json")
            with receipt_path.open("w") as output:
                subprocess.run([str(extractor), str(binary)], check=True, stdout=output)
            receipt = load_json(receipt_path)
            roots = [item for item in receipt["audit"]["packages"] if item.get("root")]
            if len(roots) != 1 or roots[0]["version"] != plan["version"]:
                raise ValueError("archive binary application version differs from plan")
            syft_path = evidence / (stem + ".syft.json")
            with syft_path.open("w") as output:
                subprocess.run([str(syft), "scan", "file:" + str(binary), "-o", "syft-json"], check=True, stdout=output)
            binary_sbom.verify_syft(load_json(syft_path), receipt)
            document = binary_sbom.sbom(binary, receipt, subject["package"], plan["source"]["commit"], source_diff)
            embedded = binary_sbom.ROOT / "cli/src/wizard/assets/shipped-composition.json"
            wizard_spec = importlib.util.spec_from_file_location("embedded_wizard", Path(__file__).with_name("verify-embedded-wizard.py"))
            wizard = importlib.util.module_from_spec(wizard_spec)
            wizard_spec.loader.exec_module(wizard)
            embedded_receipt = wizard.verify(binary, embedded.parent)
            (evidence / (stem + ".embedded-wizard.json")).write_text(json.dumps(embedded_receipt, indent=2) + "\n")
            binary_sbom.attach_wizard(document, embedded.read_bytes(), embedded_receipt)
            (evidence / (stem + ".binary.cdx.json")).write_text(json.dumps(document, indent=2) + "\n")
            archive_document = archive_sbom(document, subject, inspection)
            output_path = evidence / (stem + ".cdx.json")
            output_path.write_text(json.dumps(archive_document, indent=2) + "\n")
            records.append({**{key: subject[key] for key in ("id", "kind", "name", "target")},
                            "source": plan["source"], "digest": inspection["digest"], "binary": receipt["subject"],
                            "sbom": {"path": output_path.name, "digest": file_digest(output_path)}})
    return {"schema": "nyxid.archive-inspection.v1", "scope": "complete" if targets is None and source_diff is None else "local-investigation",
            "source": plan["source"], "subjects": records}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--plan", type=Path, required=True)
    parser.add_argument("--artifacts", type=Path, required=True)
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--extractor", type=Path, required=True)
    parser.add_argument("--syft", type=Path, required=True)
    parser.add_argument("--target", action="append", help="local subset only; final publication must omit this option")
    parser.add_argument("--source-diff", type=Path, help="local investigation only; final publication requires a clean producer checkout")
    args = parser.parse_args()
    result = inspect(load_json(args.plan), args.artifacts, args.evidence, args.extractor.resolve(), args.syft.resolve(), args.target, args.source_diff)
    (args.evidence / "archive-inspection.json").write_text(json.dumps(result, indent=2) + "\n")
    print("Inspected " + str(len(result["subjects"])) + " planned archives (" + result["scope"] + ")")
