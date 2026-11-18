#!/usr/bin/env python3
"""Copy Cargo manifests/lock with only workspace package versions normalized.

The output is copied into a fresh Docker stage, so release-only version bumps
produce identical dependency-build inputs. No third-party resolution is done.
Requires Python 3.11+ (tomllib); used only in the recipe stage, never at runtime.
"""

import re
import sys
import tomllib
from pathlib import Path

VERSION = "0.0.0"


def normalize_package(text: str) -> str:
    """Edit only [package].version, leaving dependency requirements untouched."""
    sections = re.split(r"(?m)(?=^\[)", text)
    for index, section in enumerate(sections):
        if section.startswith("[package]\n"):
            sections[index], count = re.subn(
                r'(?m)^version\s*=\s*"[^"\n]+"', f'version = "{VERSION}"', section
            )
            if count != 1:
                raise ValueError("expected one explicit [package].version")
            return "".join(sections)
    raise ValueError("missing [package]")


def check_path_requirements(value: object) -> None:
    # Today every workspace edge is path-only. Fail on a new version constraint
    # rather than silently weakening it or making --locked resolve other crates.
    if isinstance(value, dict):
        if "path" in value and "version" in value:
            raise ValueError("versioned path dependencies need explicit recipe support")
        for child in value.values():
            check_path_requirements(child)
    elif isinstance(value, list):
        for child in value:
            check_path_requirements(child)


def recipe(source: Path, destination: Path) -> None:
    workspace_text = (source / "Cargo.toml").read_text()
    workspace = tomllib.loads(workspace_text)
    check_path_requirements(workspace)
    members = workspace["workspace"]["members"]
    files = {"Cargo.toml": workspace_text}
    versions = {}
    for member in members:
        # This workspace lists its directories explicitly. Do not silently omit
        # a new glob member or write outside the recipe directory.
        if Path(member).is_absolute() or ".." in Path(member).parts or any(
            char in member for char in "*?[]"
        ):
            raise ValueError(f"expected explicit workspace member: {member}")
        path = f"{member}/Cargo.toml"
        text = (source / path).read_text()
        manifest = tomllib.loads(text)
        check_path_requirements(manifest)
        package = manifest["package"]
        if not isinstance(package["version"], str) or package["name"] in versions:
            raise ValueError("expected unique members with explicit versions")
        versions[package["name"]] = package["version"]
        files[path] = normalize_package(text)

    lock = (source / "Cargo.lock").read_text()
    blocks = re.split(r"(?m)(?=^\[\[package\]\])", lock)
    found = set()
    for index, block in enumerate(blocks):
        if not block.startswith("[[package]]"):
            continue
        package = tomllib.loads(block)["package"][0]
        name = package["name"]
        if "source" not in package:
            if name not in versions or package["version"] != versions[name]:
                raise ValueError(f"lock entry does not match workspace member: {name}")
            found.add(name)
            block = re.sub(
                r'(?m)^version = "[^"\n]+"$', f'version = "{VERSION}"', block, count=1
            )
        # Cargo may version-qualify a local edge when a registry package shares
        # its name. Source-qualified registry/git edges must remain byte-exact.
        for name, version in versions.items():
            block = block.replace(f'"{name} {version}"', f'"{name} {VERSION}"')
        blocks[index] = block
    if found != versions.keys():
        raise ValueError(f"workspace members missing from lock: {versions.keys() - found}")
    files["Cargo.lock"] = "".join(blocks)
    # Validate before writing anything, including that the lock is still TOML.
    tomllib.loads(files["Cargo.lock"])
    for path, text in files.items():
        output = destination / path
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(text)


if __name__ == "__main__":
    recipe(Path(sys.argv[1]), Path(sys.argv[2]))
