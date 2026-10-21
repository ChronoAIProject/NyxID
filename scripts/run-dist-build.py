#!/usr/bin/env python3
"""Require fresh dist outputs, exact exported source and completed locked child builds.

This produces build evidence only. Final publication still requires complete
archive/SBOM/scan/signature checks and successful required workflow jobs.
"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import shutil
import sys
import tempfile

sys.dont_write_bytecode = True
from publication_contract import file_digest, load_json, require_build_execution, validate_plan


def module(name, filename):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(filename))
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


runner = module("dist_exported_runner", "run-exported-build.py")
source = runner.source
source_builder = module("dist_source_builder", "build-release-source.py")
archives = module("dist_archive_inspector", "check-dist-artifacts.py")


def local_subjects(plan, targets):
    return [item for item in plan["subjects"] if item.get("target") in targets]


def required_children(directory, targets, identity):
    records = []
    for path in sorted(directory.glob("*.json")):
        receipt = load_json(path)
        if receipt.get("schema") != "nyxid.dist-cargo-build.v1" or receipt.get("source") != identity or \
                receipt.get("exit_code") != 0 or receipt.get("build_finished") != [True] or \
                len(receipt.get("artifacts", [])) != 1 or "--locked" not in receipt.get("command", []):
            raise ValueError("required nested Cargo build failed, was cancelled, incomplete or unlocked")
        message = receipt.get("messages", {})
        if message.get("name") != path.stem + ".jsonl" or \
                file_digest(directory / message["name"]) != "sha256:" + message.get("sha256", ""):
            raise ValueError("nested Cargo message evidence changed")
        # Verify the actual completion stream as well as the summarized result.
        events = [json.loads(line) for line in (directory / message["name"]).read_text().splitlines()
                  if line.startswith("{")]
        if [item.get("success") for item in events if item.get("reason") == "build-finished"] != [True]:
            raise ValueError("nested Cargo stream lacks successful required completion")
        records.append(receipt)
    if sorted(item.get("target") for item in records) != sorted(targets):
        raise ValueError("required nested Cargo target coverage missing, duplicate or extra")
    if {path.name for path in directory.iterdir()} != \
            {name for path in directory.glob("*.json") for name in (path.name, path.stem + ".jsonl")}:
        raise ValueError("unexpected/incomplete nested Cargo evidence")
    return {item["target"]: item for item in records}


def archive_records(plan, stage, targets, children):
    records = []
    for subject in local_subjects(plan, targets):
        path = stage / subject["name"]
        with tempfile.TemporaryDirectory(prefix="nyxid-dist-completion-") as temporary:
            inspected = archives.inspect_archive(path, subject, Path(temporary))
        binary = next(item for item in inspected["assets"] if item["path"] == subject["binary"])
        child = children[subject["target"]]["artifacts"][0]
        if not child.get("package_id", "").endswith("#nyxid-cli@" + plan["version"]) or \
                binary["sha256"] != child["sha256"]:
            raise ValueError("fresh archive binary differs from required completed Cargo output")
        checksum = path.with_name(path.name + ".sha256")
        # Pinned dist adds an extra trailing blank line to checksum files.
        if checksum.read_text() not in (inspected["digest"][7:] + " *" + path.name + ending for ending in ("\n", "\n\n")):
            raise ValueError("fresh archive checksum differs from actual bytes")
        working = stage / subject["name"][:-len(".tar.gz")]
        if working.is_symlink():
            raise ValueError("dist archive working directory is a symlink")
        if working.exists():
            assets = {item["path"]: item for item in inspected["assets"]}
            if not working.is_dir() or {item.name for item in working.iterdir()} != set(assets) or \
                    any(file_digest(working / name) != "sha256:" + item["sha256"] for name, item in assets.items()):
                raise ValueError("dist working assets differ from inspected archive members")
        records.extend([{"name": path.name, "digest": inspected["digest"]},
                        {"name": checksum.name, "digest": file_digest(checksum)}])
    return records


def verify_local(plan, directory, evidence, targets, raw_archive):
    receipt = load_json(evidence / "build-completion.json")
    if receipt.get("schema") != "nyxid.dist-build-completion.v1" or receipt.get("mode") != "local" or \
            receipt.get("source") != plan["source"] or receipt.get("targets") != sorted(targets):
        raise ValueError("required local build completion differs from expected plan")
    require_build_execution(load_json(evidence / "execution.json"), plan["source"], raw_archive)
    children = required_children(evidence / "cargo", targets, plan["source"])
    records = archive_records(plan, directory, targets, children)
    if records != receipt.get("artifacts") or \
            file_digest(evidence / "dist-manifest.json") != receipt.get("dist_manifest"):
        raise ValueError("completed local artifact/manifest bytes changed")
    return records


def fresh_staging(tree):
    """Discard directory-only cache residue, never a previous build output.

    The pinned rust-cache cleanup removes artifacts but retains directories.
    Validate both reserved staging trees before removing any directory; cached
    dependency/profile files elsewhere in target remain available to Cargo.
    """
    if (tree / "target").is_symlink():
        raise ValueError("dist requires a regular target directory")
    directories = []
    for root in (tree / "target/distrib", tree / source_builder.OUTPUT):
        if root.is_symlink():
            raise ValueError("dist requires fresh artifact staging; symlinks cannot satisfy a build")
        if root.exists():
            entries = [root, *root.rglob("*")]
            if any(path.is_symlink() or not path.is_dir() for path in entries):
                raise ValueError("dist requires fresh artifact staging; prior artifacts cannot satisfy a build")
            directories.extend(entries)
    removed = sorted(str(path.relative_to(tree)) for path in directories)
    for path in sorted(directories, key=lambda item: len(item.parts), reverse=True):
        path.rmdir()
    return removed


def run(destination, plan, evidence, mode, targets=(), dist="dist", local_artifacts=None, local_evidence=None):
    validate_plan(plan)
    if plan["producer"] != "cli" or mode not in ("local", "global"):
        raise ValueError("expected a CLI publication plan and local/global build mode")
    targets = sorted(targets)
    all_targets = sorted(item["target"] for item in plan["subjects"] if item.get("target"))
    if (mode == "local" and (not targets or len(set(targets)) != len(targets) or not set(targets) <= set(all_targets))) or \
            (mode == "global" and targets):
        raise ValueError("build targets differ from validated plan")
    destination = destination.resolve()
    evidence = evidence.resolve()
    if source.within(evidence, destination / "source"):
        raise ValueError("dist evidence must be outside source inputs")
    exported = source.verify(destination, plan["source"]["commit"], ["target"])
    if {key: exported["source"][key] for key in ("commit", "epoch")} != plan["source"]:
        raise ValueError("dist source export differs from validated plan")
    tree = destination / "source"
    stage = tree / "target/distrib"
    removed_empty_staging = fresh_staging(tree)
    evidence.mkdir(parents=True, exist_ok=False)
    (evidence / "cargo").mkdir()
    if mode == "global":
        if local_artifacts is None or local_evidence is None:
            raise ValueError("global packaging requires all completed local build evidence")
        stage.mkdir(parents=True)
        expected_names = set()
        for target in all_targets:
            records = verify_local(plan, local_artifacts, local_evidence / target, [target], exported["archive"]["sha256"])
            for item in records:
                expected_names.add(item["name"])
                shutil.copyfile(local_artifacts / item["name"], stage / item["name"])
            shutil.copyfile(local_evidence / target / "dist-manifest.json", stage / (target + "-dist-manifest.json"))
        if {path.name for path in local_artifacts.iterdir()} != expected_names or \
                {path.name for path in local_evidence.iterdir()} != set(all_targets):
            raise ValueError("local input set is incomplete or has extra subjects")
    cargo = shutil.which("cargo")
    if not cargo:
        raise ValueError("real Cargo is unavailable")
    command = [dist, "build", "--artifacts=" + mode, "--tag=" + plan["tag"], "--output-format=json"]
    command += ["--target=" + target for target in targets]
    environment = {"CARGO": str(tree / "scripts/guard-dist-cargo.py"),
                   "NYXID_DIST_REAL_CARGO": cargo, "NYXID_DIST_TARGETS": json.dumps(targets),
                   "NYXID_DIST_CARGO_RECEIPTS": str(evidence / "cargo")}
    with (evidence / "dist-manifest.json").open("x") as output:
        code = runner.run(destination, plan["source"]["commit"], evidence / "execution.json",
                          command, ["target"], environment=environment, stdout=output)
    require_build_execution(load_json(evidence / "execution.json"), plan["source"], exported["archive"]["sha256"])
    if code:
        raise ValueError("required outer dist command failed")
    if mode == "local":
        children = required_children(evidence / "cargo", targets, plan["source"])
        records = archive_records(plan, stage, targets, children)
        working_directories = {item["name"][:-len(".tar.gz")] for item in local_subjects(plan, targets)
                               if (stage / item["name"][:-len(".tar.gz")]).exists()}
        if {path.name for path in stage.iterdir()} != {item["name"] for item in records} | working_directories:
            raise ValueError("fresh local staging has unexpected outputs")
    else:
        required_children(evidence / "cargo", [], plan["source"])
        hook = load_json(tree / source_builder.OUTPUT / "source-archive.json")
        actual = source_builder.inspect(stage / "source.tar.gz", exported)
        if hook.get("schema") != "nyxid.release-source.v1" or hook.get("source") != exported["source"] or \
                hook.get("raw_archive") != exported["archive"] or hook.get("subject") != actual or \
                hook.get("post_build_source_verified") is not True or \
                hook.get("export_receipt_sha256") != source.digest(destination / "source-export.json") or \
                hook.get("checksum", {}).get("sha256") != source.digest(stage / "source.tar.gz.sha256") or \
                (stage / "source.tar.gz.sha256").read_text() != actual["sha256"] + " *source.tar.gz\n":
            raise ValueError("required fresh source hook did not complete with exact source bytes")
        shutil.copyfile(tree / source_builder.OUTPUT / "source-archive.json", evidence / "source-archive.json")
        expected_names = {item["name"] for item in plan["subjects"] if item["name"] != "releases.json"}
        generated_manifests = {target + "-dist-manifest.json" for target in all_targets}
        if {path.name for path in stage.iterdir()} != expected_names | generated_manifests:
            raise ValueError("global staging omits/adds required publication files")
        records = [{"name": name, "digest": file_digest(stage / name)} for name in sorted(expected_names)]
    receipt = {"schema": "nyxid.dist-build-completion.v1", "source": plan["source"],
               "mode": mode, "targets": targets, "artifacts": records,
               "removed_empty_staging_directories": removed_empty_staging,
               "dist_manifest": file_digest(evidence / "dist-manifest.json")}
    with (evidence / "build-completion.json").open("x") as output:
        json.dump(receipt, output, indent=2)
        output.write("\n")
    upload = evidence / "artifacts"
    upload.mkdir()
    for item in records:
        shutil.copyfile(stage / item["name"], upload / item["name"])
        if file_digest(upload / item["name"]) != item["digest"]:
            raise ValueError("upload copy differs from completed artifact bytes")
    return receipt


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--export", dest="destination", type=Path, required=True)
    parser.add_argument("--plan", type=Path, required=True)
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--mode", choices=("local", "global"), required=True)
    parser.add_argument("--target", action="append", default=[])
    parser.add_argument("--dist", default="dist")
    parser.add_argument("--local-artifacts", type=Path)
    parser.add_argument("--local-evidence", type=Path)
    args = parser.parse_args()
    print(json.dumps(run(args.destination, load_json(args.plan), args.evidence, args.mode, args.target,
                         args.dist, args.local_artifacts, args.local_evidence)))
