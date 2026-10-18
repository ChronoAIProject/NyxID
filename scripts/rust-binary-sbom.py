#!/usr/bin/env python3
"""Generate CycloneDX from metadata extracted from the actual final Rust binary.

The receipt is produced by the locked security/artifact-tools parser. Every
generation rehashes the subject; a source Cargo.lock is never a substitute.
"""
import argparse
import base64
import hashlib
import json
from pathlib import Path
import re
import uuid
from shipped_composition_sbom import components as composition_components

ROOT = Path(__file__).resolve().parents[1]


def local_pedigree(package, commit):
    catalog = json.loads((ROOT / "security/rust-local-provenance.json").read_text())
    record = next((item for item in catalog["packages"] if item["name"] == package["name"] and item["version"] == package["version"]), None)
    if not record:
        return None
    directory = ROOT / record["path"]
    actual = []
    for path in sorted(directory.rglob("*")):
        if path.is_symlink():
            raise ValueError("vendored pedigree source cannot contain symlinks")
        if path.is_file():
            actual.append({"path": path.relative_to(directory).as_posix(), "sha256": hashlib.sha256(path.read_bytes()).hexdigest()})
    if actual != record["files"]:
        raise ValueError("vendored pedigree differs from reviewed patched source")
    upstream = record["upstream"]
    return {"ancestors": [{"type": "library", "name": upstream["name"], "version": upstream["version"], "purl": upstream["purl"],
        "hashes": [{"alg": "SHA-256", "content": upstream["sha256"]}],
        "externalReferences": [{"type": "distribution", "url": upstream["archive_url"]}, {"type": "vcs", "url": upstream["repository"]}]}],
        "patches": [{"type": "unofficial", "diff": {"text": {"contentType": "text/x-diff", "encoding": "base64", "content": base64.b64encode(record["patch"]["diff"].encode()).decode()}},
            "resolves": [{"type": "defect", "name": "Rust E0365", "description": record["patch"]["description"],
                "references": ["https://github.com/ChronoAIProject/NyxID/blob/" + commit + "/" + record["patch"]["documentation"]]}]}]}


def verify_syft(catalog, receipt):
    if catalog.get("descriptor", {}).get("name") != "syft" or catalog["descriptor"].get("version") != "1.54.1" or \
            catalog.get("source", {}).get("type") != "file" or \
            {"algorithm": "sha256", "value": receipt["subject"]["sha256"]} not in catalog["source"].get("metadata", {}).get("digests", []):
        raise ValueError("Syft catalog does not bind the actual binary subject/tool")
    expected = {(item["name"], item["version"], item["source"]) for item in receipt["audit"]["packages"] if item.get("kind", "runtime") == "runtime"}
    actual = {(item["metadata"]["name"], item["metadata"]["version"], item["metadata"]["source"])
              for item in catalog.get("artifacts", []) if item.get("foundBy") == "cargo-auditable-binary-cataloger"
              and item.get("metadataType") == "rust-cargo-audit-entry"}
    if actual != expected:
        raise ValueError("Syft runtime inventory differs from validated compiled metadata")


def sbom(binary, receipt, expected_package, commit, source_diff=None):
    subject = receipt.get("subject", {})
    hasher = hashlib.sha256()
    with binary.open("rb") as source:
        for block in iter(lambda: source.read(64 * 1024), b''):
            hasher.update(block)
    digest = hasher.hexdigest()
    if receipt.get("schema") != "nyxid.rust-binary-info.v1" or subject.get("sha256") != digest or \
            subject.get("size") != binary.stat().st_size or subject.get("name") != binary.name:
        raise ValueError("binary metadata receipt does not match actual subject bytes")
    if not re.fullmatch("[0-9a-f]{40}", commit):
        raise ValueError("full source commit is required")
    audit = receipt["audit"]
    packages = audit["packages"]
    roots = [index for index, item in enumerate(packages) if item.get("root")]
    if len(roots) != 1 or packages[roots[0]]["name"] != expected_package or len(packages) < 2:
        raise ValueError("expected final application dependency metadata is absent")
    refs = ["rust:" + str(index) + ":" + item["name"] + "@" + item["version"] for index, item in enumerate(packages)]
    components, dependencies = [], []
    for index, package in enumerate(packages):
        kind = package.get("kind", "runtime")
        if kind not in ("runtime", "build"):
            raise ValueError("unknown compiled dependency kind")
        children = package.get("dependencies", [])
        if any(type(child) is not int or not 0 <= child < len(packages) for child in children):
            raise ValueError("invalid compiled dependency reference")
        component = {"type": "application" if package.get("root") else "library", "bom-ref": refs[index],
            "name": package["name"], "version": package["version"],
            "properties": [{"name": "nyxid:compiled-dependency-kind", "value": kind},
                           {"name": "nyxid:compiled-source-kind", "value": package["source"]}]}
        # The audit format intentionally omits registry/git URLs and local
        # paths. A matching name/version alone cannot establish registry origin.
        if package["source"] == "crates.io":
            component["purl"] = "pkg:cargo/" + package["name"] + "@" + package["version"]
        elif package["source"] == "local":
            component["externalReferences"] = [{"type": "vcs", "url": "https://github.com/ChronoAIProject/NyxID/tree/" + commit}]
            pedigree = local_pedigree(package, commit)
            if pedigree:
                component["pedigree"] = pedigree
        else:
            raise ValueError("compiled source requires an authenticated origin mapping: " + package["source"])
        components.append(component)
        dependencies.append({"ref": refs[index], "dependsOn": [refs[child] for child in children]})
    ref = "sha256:" + digest
    document = {"bomFormat": "CycloneDX", "specVersion": "1.6", "version": 1,
        "serialNumber": "urn:uuid:" + str(uuid.uuid5(uuid.NAMESPACE_URL, ref)),
        "metadata": {"component": {"type": "application", "bom-ref": ref, "name": binary.name,
            "hashes": [{"alg": "SHA-256", "content": digest}],
            "properties": [{"name": "nyxid:source-commit", "value": commit},
                {"name": "nyxid:inventory-source", "value": "final binary cargo-auditable section"},
                {"name": "nyxid:audit-format", "value": str(audit.get("format", 0))},
                {"name": "nyxid:inventory-precision", "value": "Stable Cargo metadata may conservatively include extra dependencies; this is not per-crate emitted-byte attribution."}]}},
        "components": components, "dependencies": [{"ref": ref, "dependsOn": [refs[roots[0]]]}, *dependencies]}
    if source_diff:
        document["metadata"]["component"]["properties"].append({"name": "nyxid:source-diff-sha256", "value": hashlib.sha256(source_diff.read_bytes()).hexdigest()})
    return document


def attach_wizard(document, seal_bytes, embedded):
    seal = json.loads(seal_bytes)
    component = document["metadata"]["component"]
    subject_hash = component["hashes"][0]["content"]
    seal_hash = hashlib.sha256(seal_bytes).hexdigest()
    if embedded.get("schema") != "nyxid.embedded-wizard.v1" or embedded["subject"]["sha256"] != subject_hash or embedded["seal_sha256"] != seal_hash:
        raise ValueError("embedded wizard receipt does not bind this subject/seal")
    records, refs = composition_components(seal, "wizard:")
    wizard_ref = "wizard:sha256:" + seal_hash
    document["components"].append({"type": "application", "bom-ref": wizard_ref, "name": "nyxid-embedded-wizard",
        "properties": [{"name": "nyxid:seal-sha256", "value": seal_hash}]})
    document["components"].extend(records)
    document["dependencies"].append({"ref": wizard_ref, "dependsOn": refs})
    document["dependencies"][0]["dependsOn"].append(wizard_ref)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--receipt", type=Path, required=True)
    parser.add_argument("--syft", type=Path, required=True, help="pinned Syft file scan in syft-json format")
    parser.add_argument("--package", required=True)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--source-diff", type=Path, help="local working-patch receipt; omit only for a clean release checkout")
    parser.add_argument("--wizard-seal", type=Path)
    parser.add_argument("--embedded-wizard", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    receipt = json.loads(args.receipt.read_text())
    verify_syft(json.loads(args.syft.read_text()), receipt)
    document = sbom(args.binary, receipt, args.package, args.commit, args.source_diff)
    if bool(args.wizard_seal) != bool(args.embedded_wizard) or (args.package == "nyxid-cli" and not args.wizard_seal):
        raise ValueError("CLI SBOM requires its verified embedded wizard seal and receipt")
    if args.wizard_seal:
        attach_wizard(document, args.wizard_seal.read_bytes(), json.loads(args.embedded_wizard.read_text()))
    args.output.write_text(json.dumps(document, indent=2) + "\n")
    print("Final binary subject and compiled dependency SBOM bound")
