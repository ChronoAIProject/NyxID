#!/usr/bin/env python3
"""Fail CI when generated workflow or image edits reintroduce mutable inputs."""
import argparse
import json
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parents[1]


def check_image(image, pins, where):
    if not isinstance(image, str) or not re.fullmatch(r"[^\s@]+@sha256:[0-9a-f]{64}", image):
        return [where + ": unpinned consumed image " + str(image)]
    if image not in pins:
        return [where + ": image pin lacks resolution receipt " + image]
    return []


def check_containers(value, pins, where):
    errors = []
    if isinstance(value, dict):
        for key, child in value.items():
            if key == "container":
                image = child.get("image") if isinstance(child, dict) else child
                errors.extend(check_image(image, pins, where + ".container"))
            else:
                errors.extend(check_containers(child, pins, where + "." + key))
    elif isinstance(value, list):
        for index, child in enumerate(value):
            errors.extend(check_containers(child, pins, f"{where}[{index}]"))
    return errors


def check(root, plan=None):
    errors = []
    receipts = json.loads((root / "security/build-input-pins.json").read_text())
    action_pins = {(a["repository"].lower(), a["sha"]) for a in receipts["actions"]}
    image_pins = {a["image"] + "@" + a["digest"] for a in receipts["images"]}
    # Use Cargo's own TOML parser; do not infer dist containers from Dockerfiles
    # or from permissive regular expressions over the source manifest.
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--locked", "--no-deps", "--format-version=1"], cwd=root, text=True))
    if metadata["metadata"].get("dist", {}).get("cargo-auditable") is not True:
        errors.append("Cargo dist metadata: shipped binaries require cargo-auditable")
    errors.extend(check_containers(metadata["metadata"].get("dist", {}), image_pins, "Cargo dist metadata"))
    if plan is not None:
        errors.extend(check_containers(plan, image_pins, "generated dist plan"))
    fixtures = json.loads((root / "security/test-images.json").read_text())["images"]
    for role, image in fixtures.items():
        errors.extend(check_image(image, image_pins, "test fixture " + role))
    mongo = (root / ".github/scripts/start-test-mongodb.sh").read_text()
    for image in re.findall(r"\bmongo:[^\s\\]+", mongo):
        errors.extend(check_image(image, image_pins, "Mongo test fixture"))
    for path in sorted(path for path in (root / ".github/workflows").iterdir()
                       if path.suffix in (".yml", ".yaml")):
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
        # cargo-dist also emits an unchecked latest installer for auditable.
        # Reject dot and bracket access before regeneration can consume it.
        if re.search(r"matrix\s*(?:\.\s*(?:install_dist|install_cargo_auditable|packages_install)\b|\[\s*['\"](?:install_dist|install_cargo_auditable|packages_install)['\"]\s*\])", text):
            errors.append(str(path.relative_to(root)) + ": executable generated installer/package input")
        if "sh.rustup.rs" in text or "cargo-dist-installer.sh" in text:
            errors.append(str(path.relative_to(root)) + ": unchecked bootstrap installer")
        if re.search(r"cargo (?:auditable )?build(?![^\n]*(?:--locked|--frozen))", text):
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
            if image not in stages and image != "scratch" and not dynamic_test:
                if not re.fullmatch(r"[^\s@]+@sha256:[0-9a-f]{64}", image):
                    errors.append(name + ": unpinned base " + image)
                elif image not in image_pins:
                    errors.append(name + ": base pin lacks resolution receipt " + image)
            if stage:
                stages.add(stage)
        if "sh.rustup.rs" in text:
            errors.append(name + ": unchecked Rust installer")
        if re.search(r"cargo (?:auditable )?build(?![^\n]*(?:--locked|--frozen))", text):
            errors.append(name + ": Rust build does not enforce lockfile")
        commands = '\n'.join(line for line in text.splitlines() if not line.lstrip().startswith('#'))
        if name not in ("cli/tests/machine_context_spike/Dockerfile",) and re.search(r"\bcargo build\b", commands):
            errors.append(name + ": shipped Rust build omits auditable metadata")
    for name in ("release.yml", "release-cache.yml"):
        text = (root / ".github/workflows" / name).read_text()
        if "scripts/bootstrap-build-tool.py cargo-dist" not in text:
            errors.append(name + ": missing checksummed dist bootstrap")
        if "scripts/bootstrap-build-tool.py cargo-auditable" not in text:
            errors.append(name + ": missing checksummed cargo-auditable bootstrap")
    return errors


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=ROOT)
    parser.add_argument("--dist-plan", type=Path, help="actual cargo-dist JSON plan to validate before matrix consumption")
    args = parser.parse_args()
    errors = check(args.root, json.loads(args.dist_plan.read_text()) if args.dist_plan else None)
    if errors:
        raise SystemExit("\n".join(errors))
    print("Action/base/fixture pins, Cargo dist containers, resolution receipts, "
          "locked Rust builds and generated release bootstrap policy passed")


if __name__ == "__main__":
    main()
