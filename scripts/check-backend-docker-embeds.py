#!/usr/bin/env python3
"""Verify that every production backend build input is staged by Docker."""

from __future__ import annotations

import argparse
import fnmatch
import shlex
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
    text = path.read_text(encoding="utf-8").replace("\\\n", " ")
    separator = text.find(": ")
    if separator < 0:
        raise ValueError(f"could not parse Cargo dep-info: {path}")
    dependencies: list[Path] = []
    for raw in shlex.split(text[separator + 2 :]):
        dependency = Path(raw)
        if not dependency.is_absolute():
            dependency = REPO_ROOT / dependency
        dependencies.append(dependency.resolve())
    return dependencies


def builder_copy_sources(path: Path) -> list[Path]:
    logical_lines = path.read_text(encoding="utf-8").replace("\\\n", " ").splitlines()
    sources: list[Path] = []
    saw_builder = False
    for raw_line in logical_lines:
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue
        instruction = line.split(None, 1)[0].upper()
        if instruction == "FROM":
            if saw_builder:
                break
            saw_builder = True
            continue
        if not saw_builder or instruction != "COPY":
            continue
        args = shlex.split(line.split(None, 1)[1], comments=True)
        args = [arg for arg in args if not arg.startswith("--")]
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


def check(dep_info: Path, dockerfile: Path, dockerignore: Path) -> int:
    dependencies = parse_dep_info(dep_info)
    copy_sources = builder_copy_sources(dockerfile)
    ignore_patterns = dockerignore_patterns(dockerignore)
    failures: list[str] = []
    checked = 0

    for dependency in dependencies:
        try:
            relative = dependency.relative_to(REPO_ROOT)
        except ValueError:
            continue
        if relative.parts[:2] == ("backend", ".git"):
            # build.rs reads git metadata locally; the image receives the hash
            # through NYXID_GIT_HASH because .git is intentionally not staged.
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
    parser.add_argument("--dep-info", type=Path, default=DEFAULT_DEP_INFO)
    parser.add_argument("--dockerfile", type=Path, default=DEFAULT_DOCKERFILE)
    parser.add_argument("--dockerignore", type=Path, default=DEFAULT_DOCKERIGNORE)
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return 0
    try:
        return check(args.dep_info, args.dockerfile, args.dockerignore)
    except (OSError, ValueError) as error:
        print(f"Backend Docker embed guard failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
