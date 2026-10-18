#!/usr/bin/env python3
"""Fail CI when generated workflow or image edits reintroduce mutable inputs."""
import argparse
import json
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parents[1]


def check(root):
    errors = []
    receipts = json.loads((root / "security/build-input-pins.json").read_text())
    action_pins = {(a["repository"].lower(), a["sha"]) for a in receipts["actions"]}
    image_pins = {a["image"] + "@" + a["digest"] for a in receipts["images"]}
    for path in sorted((root / ".github/workflows").glob("*.yml")):
        text = path.read_text()
        for match in re.finditer(r"uses:\s*([^\s#]+)", text):
            value = match[1].strip('"\'')
            if value.startswith("./"):
                continue
            name, separator, ref = value.partition("@")
            repo = "/".join(name.split("/")[:2]).lower()
            if not separator or not re.fullmatch(r"[0-9a-f]{40}", ref):
                errors.append(str(path.relative_to(root)) + ": unpinned Action " + value)
            elif (repo, ref) not in action_pins:
                errors.append(str(path.relative_to(root)) + ": Action pin lacks resolution receipt " + value)
        if re.search(r"matrix\.(?:install_dist|packages_install)", text):
            errors.append(str(path.relative_to(root)) + ": executable generated installer/package input")
        if "sh.rustup.rs" in text or "cargo-dist-installer.sh" in text:
            errors.append(str(path.relative_to(root)) + ": unchecked bootstrap installer")
        if re.search(r"cargo build(?![^\n]*(?:--locked|--frozen))", text):
            errors.append(str(path.relative_to(root)) + ": Rust build does not enforce lockfile")
    paths = subprocess.check_output(["git", "ls-files", "*Dockerfile*"], cwd=root, text=True).splitlines()
    for name in paths:
        path = root / name
        text = path.read_text()
        stages = set()
        for line in text.splitlines():
            match = re.match(r"FROM\s+(\S+)(?:\s+AS\s+(\S+))?", line, re.I)
            if not match:
                continue
            image, stage = match.groups()
            # These two test images consume the exact image just built by CI;
            # production inputs cannot use this exception.
            dynamic_test = name in ("cli/tests/Dockerfile.machine", "cli/tests/Dockerfile.updater-browser") and image == "${MACHINE_IMAGE}"
            published_fixture = name == "cli/tests/machine_context_spike/Dockerfile" and re.fullmatch(r"ghcr.io/chronoaiproject/nyxid/nyxid-node-machine@sha256:[0-9a-f]{64}", image)
            if image not in stages and image != "scratch" and not dynamic_test and not published_fixture:
                if not re.fullmatch(r"[^\s@]+@sha256:[0-9a-f]{64}", image):
                    errors.append(name + ": unpinned base " + image)
                elif image not in image_pins:
                    errors.append(name + ": base pin lacks resolution receipt " + image)
            if stage:
                stages.add(stage)
        if "sh.rustup.rs" in text:
            errors.append(name + ": unchecked Rust installer")
        if re.search(r"cargo build(?![^\n]*(?:--locked|--frozen))", text):
            errors.append(name + ": Rust build does not enforce lockfile")
    for name in ("release.yml", "release-cache.yml"):
        text = (root / ".github/workflows" / name).read_text()
        if "scripts/bootstrap-build-tool.py cargo-dist" not in text:
            errors.append(name + ": missing checksummed dist bootstrap")
    return errors


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=ROOT)
    args = parser.parse_args()
    errors = check(args.root)
    if errors:
        raise SystemExit("\n".join(errors))
    print("Action/base pins, resolution receipts, locked Rust builds and generated release bootstrap policy passed")


if __name__ == "__main__":
    main()
