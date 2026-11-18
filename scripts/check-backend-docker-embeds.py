#!/usr/bin/env python3
"""Verify that every production backend build input is staged by Docker."""

from __future__ import annotations

import argparse
import fnmatch
import json
import shlex
import subprocess
import sys
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parent.parent
# Match backend/Dockerfile's builder command. This guard intentionally covers
# builder-stage compile inputs, not assets copied into the runtime stage.
DEFAULT_DEP_INFO = REPO_ROOT / "target/release/nyxid-server.d"
DEFAULT_DOCKERFILE = REPO_ROOT / "backend/Dockerfile"
DEFAULT_DOCKERIGNORE = REPO_ROOT / ".dockerignore"


def parse_dep_info(path: Path) -> list[Path]:
    if not path.is_file():
        raise ValueError(
            f"missing {path.relative_to(REPO_ROOT)}; run "
            "`cargo build --release -p nyxid --bin nyxid-server --features gcp-kms` first"
        )
    # rustc unit dep-info also contains other targets and empty phony rules.
    # Every target in this file has the same inputs; read only its first rule.
    lines = path.read_text(encoding="utf-8").replace("\\\n", " ").splitlines()
    if not lines:
        raise ValueError(f"empty Cargo dep-info: {path}")
    text = lines[0]
    separator = text.find(": ")
    if separator < 0:
        raise ValueError(f"could not parse Cargo dep-info: {path}")
    dependencies: list[Path] = []
    for raw in shlex.split(text[separator + 2 :]):
        dependency = Path(raw)
        if not dependency.is_absolute():
            dependency = REPO_ROOT / dependency
        dependencies.append(dependency.resolve())
    if not dependencies:
        raise ValueError(f"Cargo dep-info contains no inputs: {path}")
    return dependencies


def cargo_check_inputs(path: Path) -> list[Path]:
    """Use the artifacts from this successful Cargo invocation, including fresh ones.

    `check` has no top-level binary .d. Its JSON stream identifies the current
    units, avoiding stale .d files left by removed sources or feature sets.
    """
    messages = [json.loads(line) for line in path.read_text().splitlines() if line.strip()]
    finished = [message for message in messages if message.get("reason") == "build-finished"]
    if len(finished) != 1 or finished[0].get("success") is not True:
        raise ValueError("Cargo messages must contain one successful build-finished event")
    packages = {}
    dependencies = set()
    backend = []
    for message in messages:
        if message.get("reason") != "compiler-artifact":
            continue
        manifest = Path(message["manifest_path"]).resolve()
        if not manifest.is_relative_to(REPO_ROOT):
            continue
        packages[message["package_id"]] = manifest.parent
        target = message["target"]
        if manifest == REPO_ROOT / "backend/Cargo.toml" and target["kind"] == ["bin"]:
            backend.append(message)
        if target["kind"] == ["custom-build"]:
            executable, = message["filenames"]
            # Cargo copies the hashed executable to build-script-build. The
            # containing directory identifies its exact compilation unit.
            directory = Path(executable).parent
            unit_hash = directory.name.rsplit("-", 1)[1]
            dep_info = directory / f"build_script_build-{unit_hash}.d"
        else:
            metadata = [Path(name) for name in message["filenames"] if name.endswith(".rmeta")]
            if len(metadata) != 1 or not metadata[0].name.startswith("lib"):
                raise ValueError(f"expected one cargo check metadata artifact for {target['name']}")
            dep_info = metadata[0].with_name(metadata[0].stem[3:] + ".d")
        dependencies.update(parse_dep_info(dep_info))
    if (len(backend) != 1 or backend[0]["target"]["name"] != "nyxid-server"
            or "gcp-kms" not in backend[0]["features"]
            or backend[0]["profile"]["opt_level"] != "3"
            or backend[0]["profile"]["test"]):
        raise ValueError("expected the production nyxid-server release/gcp-kms check artifact")
    for message in messages:
        if message.get("reason") != "build-script-executed" or message["package_id"] not in packages:
            continue
        # Cargo's aggregate .d also includes files watched by build scripts.
        # The unit .d alone would miss assets read to generate OUT_DIR content.
        output = Path(message["out_dir"]).parent / "output"
        for line in output.read_text().splitlines():
            if line.startswith(("cargo:rerun-if-changed=", "cargo::rerun-if-changed=")):
                dependency = Path(line.split("=", 1)[1])
                if not dependency.is_absolute():
                    dependency = packages[message["package_id"]] / dependency
                dependencies.add(dependency.resolve())
    return sorted(dependencies)


def git_stamp_inputs() -> set[Path]:
    """Only the Git metadata deliberately replaced by the image's runtime stamp."""
    names = ["HEAD", "logs/HEAD"]
    reference = subprocess.run(["git", "symbolic-ref", "-q", "HEAD"], cwd=REPO_ROOT,
                               capture_output=True, text=True)
    if reference.returncode == 0:
        names.append(reference.stdout.strip())
    paths = set()
    for name in names:
        result = subprocess.run(["git", "rev-parse", "--git-path", name],
                                cwd=REPO_ROOT, capture_output=True, text=True)
        if result.returncode == 0:
            path = (REPO_ROOT / result.stdout.strip()).resolve()
            # A missing HEAD reflog is the build script's deliberate rerun
            # fallback when Git has no existing reflog or loose branch ref.
            if path.is_file() or name == "logs/HEAD":
                paths.add(path)
    return paths


def builder_copy_sources(path: Path) -> list[Path]:
    """Build-context COPY sources of the `builder` stage and the stages before it.

    The builder may extend earlier stages (dependency recipe and dependency
    build), so every context COPY up to the end of `builder` is a compile
    input. `COPY --from=<stage>` reads another stage, not the build context.
    """
    logical_lines = path.read_text(encoding="utf-8").replace("\\\n", " ").splitlines()
    sources: list[Path] = []
    in_builder = False
    saw_builder = False
    for raw_line in logical_lines:
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue
        instruction = line.split(None, 1)[0].upper()
        if instruction == "FROM":
            if in_builder:
                break
            words = line.split()
            in_builder = len(words) >= 4 and words[-2].upper() == "AS" and words[-1] == "builder"
            saw_builder = saw_builder or in_builder
            continue
        if instruction != "COPY":
            continue
        raw_args = shlex.split(line.split(None, 1)[1], comments=True)
        if any(arg.startswith("--from=") for arg in raw_args):
            continue
        args = [arg for arg in raw_args if not arg.startswith("--")]
        if len(args) < 2 or args[0].startswith("["):
            raise ValueError(f"unsupported builder COPY syntax: {raw_line}")
        sources.extend((REPO_ROOT / source).resolve() for source in args[:-1])
    if not saw_builder:
        raise ValueError(f"no builder stage found in {path}")
    return sources


def dockerignore_patterns(path: Path) -> list[tuple[bool, str]]:
    patterns: list[tuple[bool, str]] = []
    for raw_line in path.read_text(encoding="utf-8").splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue
        included = line.startswith("!")
        pattern = line[1:] if included else line
        patterns.append((included, pattern.lstrip("/")))
    return patterns


def pattern_matches(path: str, pattern: str) -> bool:
    if pattern.endswith("/"):
        prefix = pattern.rstrip("/")
        return path == prefix or path.startswith(f"{prefix}/")
    if "/" in pattern:
        return fnmatch.fnmatchcase(path, pattern)
    return any(fnmatch.fnmatchcase(part, pattern) for part in path.split("/"))


def is_ignored(path: Path, patterns: list[tuple[bool, str]]) -> bool:
    relative = path.relative_to(REPO_ROOT).as_posix()
    ignored = False
    for included, pattern in patterns:
        if pattern_matches(relative, pattern):
            ignored = not included
    return ignored


def is_staged(dependency: Path, copy_sources: list[Path]) -> bool:
    for source in copy_sources:
        if dependency == source:
            return True
        try:
            dependency.relative_to(source)
        except ValueError:
            continue
        return True
    return False


def check(dep_info: Path, dockerfile: Path, dockerignore: Path, cargo_messages: Path | None = None) -> int:
    dependencies = cargo_check_inputs(cargo_messages) if cargo_messages else parse_dep_info(dep_info)
    stamp_inputs = git_stamp_inputs()
    # The build scripts emit these impossible child paths if Git cannot return
    # the reflog path. They force a rerun and are not production source inputs.
    stamp_inputs.update((REPO_ROOT / package / "build.rs/git-stamp-unavailable").resolve()
                        for package in ("backend", "cli"))
    copy_sources = builder_copy_sources(dockerfile)
    ignore_patterns = dockerignore_patterns(dockerignore)
    failures: list[str] = []
    checked = 0

    for dependency in dependencies:
        try:
            relative = dependency.relative_to(REPO_ROOT)
        except ValueError:
            continue
        if dependency in stamp_inputs:
            continue
        checked += 1
        if not dependency.exists():
            failures.append(f"missing build dependency: {relative}")
        elif not is_staged(dependency, copy_sources):
            failures.append(f"not covered by a builder COPY: {relative}")
        elif is_ignored(dependency, ignore_patterns):
            failures.append(f"excluded by .dockerignore: {relative}")

    if failures:
        print("Backend Docker staging does not cover production build inputs:", file=sys.stderr)
        for failure in failures:
            print(f"  - {failure}", file=sys.stderr)
        return 1
    print(f"Backend Docker staging covers all {checked} production build inputs.")
    return 0


def self_test() -> None:
    patterns = [(False, "docs/"), (True, "docs/kept.md"), (False, "*.pem")]
    assert is_ignored(REPO_ROOT / "docs/dropped.md", patterns)
    assert not is_ignored(REPO_ROOT / "docs/kept.md", patterns)
    assert is_ignored(REPO_ROOT / "keys/private.pem", patterns)
    assert pattern_matches("backend/prompts/direct/nyxid.md", "backend/prompts/")
    assert not pattern_matches("frontend/prompts/a.md", "backend/prompts/")

    source_dir = REPO_ROOT / "backend/src"
    assert is_staged(source_dir / "main.rs", [source_dir])
    assert not is_staged(REPO_ROOT / "skills/nyxid/SKILL.md", [source_dir])
    print("Backend Docker embed guard self-test passed.")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--self-test", action="store_true")
    inputs = parser.add_mutually_exclusive_group()
    inputs.add_argument("--dep-info", type=Path, default=DEFAULT_DEP_INFO)
    inputs.add_argument("--cargo-messages", type=Path)
    parser.add_argument("--dockerfile", type=Path, default=DEFAULT_DOCKERFILE)
    parser.add_argument("--dockerignore", type=Path, default=DEFAULT_DOCKERIGNORE)
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return 0
    try:
        return check(args.dep_info, args.dockerfile, args.dockerignore, args.cargo_messages)
    except (OSError, ValueError) as error:
        print(f"Backend Docker embed guard failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
