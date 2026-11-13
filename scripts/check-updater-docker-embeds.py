#!/usr/bin/env python3
"""Fail when the machine-updater image drops a file rustc actually compiled.

This follows scripts/check-backend-docker-embeds.py: the source list is
rustc dep-info from the production and test compiles, not a Rust parser.
The updater image stages those files through scratch stages and COPY --from.
A scan that stops at the builder line would also see the fixture stage,
which is textually above `builder` and is not a production input.

Workspace manifests and Cargo.lock are cargo inputs, not rustc inputs, so
they are checked on the dependency-recipe stage separately. Run from the
repository root. Production dep-info comes from
`cargo build --release -p nyxid-machine-updater`. Test dep-info comes from
`cargo test --release -p nyxid-machine-updater --no-run`. Pass every unit
.d file (the binary, the library, and nyxid-machine). A later test compile
in the same target directory overwrites deps/*.d with cfg(test) paths.
"""

from __future__ import annotations

import argparse
import fnmatch
import shlex
import sys
from pathlib import Path

try:
    import tomllib
except ModuleNotFoundError:
    sys.exit("check-updater-docker-embeds.py requires Python 3.11+ (tomllib).")


REPO_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_DOCKERFILE = REPO_ROOT / "cli/Dockerfile.machine-updater"
DEFAULT_DOCKERIGNORE = REPO_ROOT / ".dockerignore"
PRODUCTION_STAGE = "builder"
TEST_STAGE = "tests-build"
BROAD_COPIES = {
    "backend/",
    "cli/",
    "cloud-auth/",
    "service-adapters/",
    "permissions/",
    "mcp-demo/",
    "nyxid-crypto/",
    "machine/",
    "machine-updater/",
}
CONTAINER_SOURCE_PREFIX = "/build/"


def parse_dep_info(path: Path) -> list[Path]:
    """Return source files from the first makefile rule of a rustc .d file."""
    if not path.is_file():
        raise ValueError(f"missing dep-info {path}")
    text = path.read_text(encoding="utf-8").replace("\\\n", " ")
    rule = text.splitlines()[0] if text.splitlines() else ""
    separator = rule.find(": ")
    if separator < 0:
        raise ValueError(f"could not parse Cargo dep-info: {path}")
    dependencies: list[Path] = []
    for raw in shlex.split(rule[separator + 2 :]):
        dependencies.append(normalize_dep_path(raw))
    return dependencies


def normalize_dep_path(raw: str) -> Path:
    """Map host and image paths onto this checkout.

    Image builds run with WORKDIR /build. Host builds emit repo-relative or
    absolute host paths. Registry and sysroot files stay absolute so the
    caller can skip them.
    """
    if raw.startswith(CONTAINER_SOURCE_PREFIX):
        return (REPO_ROOT / raw.removeprefix(CONTAINER_SOURCE_PREFIX)).resolve()
    path = Path(raw)
    if not path.is_absolute():
        path = REPO_ROOT / path
    return path.resolve()


def repo_file(path: Path) -> Path | None:
    try:
        relative = path.relative_to(REPO_ROOT)
    except ValueError:
        return None
    if relative.parts[:1] == ("target",):
        return None
    return relative


def stages(text: str) -> dict[str, dict[str, object]]:
    found: dict[str, dict[str, object]] = {}
    current: str | None = None
    for raw in text.replace("\\\n", "\n").splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        instruction = line.split(None, 1)[0].upper()
        if instruction == "FROM":
            words = line.split()
            if len(words) < 4 or words[-2].upper() != "AS":
                raise ValueError(f"unnamed stage: {line}")
            current = words[-1]
            found[current] = {"parent": " ".join(words[1:-2]), "copies": [], "from": []}
            continue
        if instruction != "COPY" or current is None:
            continue
        args = shlex.split(line.split(None, 1)[1], comments=True)
        from_stage = next((arg[7:] for arg in args if arg.startswith("--from=")), None)
        paths = [arg for arg in args if not arg.startswith("--")]
        if len(paths) < 2:
            raise ValueError(f"unsupported COPY: {line}")
        if from_stage:
            found[current]["from"].append(from_stage)
        else:
            found[current]["copies"].extend(paths[:-1])
    return found


def reachable_copies(parsed: dict[str, dict[str, object]], stage: str, seen: set[str] | None = None) -> list[str]:
    """Context COPY sources that land in `stage`, following COPY --from."""
    if seen is None:
        seen = set()
    if stage in seen or stage not in parsed:
        return []
    seen.add(stage)
    info = parsed[stage]
    sources = list(info["copies"])
    for upstream in [info["parent"], *info["from"]]:
        sources.extend(reachable_copies(parsed, upstream, seen))
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


def is_ignored(relative: Path, patterns: list[tuple[bool, str]]) -> bool:
    ignored = False
    for included, pattern in patterns:
        if pattern_matches(relative.as_posix(), pattern):
            ignored = not included
    return ignored


def is_staged(relative: Path, copy_sources: list[str]) -> bool:
    candidate = relative.as_posix()
    for source in copy_sources:
        source = source.rstrip("/")
        if candidate == source or candidate.startswith(source + "/"):
            return True
    return False


def workspace_recipe_inputs() -> list[str]:
    manifest = tomllib.loads((REPO_ROOT / "Cargo.toml").read_text(encoding="utf-8"))
    members = manifest["workspace"]["members"]
    return ["Cargo.toml", "Cargo.lock", "scripts/docker-dependency-recipe.py"] + [
        f"{member}/Cargo.toml" for member in members
    ]


def structural_failures(text: str, parsed: dict[str, dict[str, object]]) -> list[str]:
    failures: list[str] = []
    if "--mount=type=cache" in text:
        failures.append("cache mounts are not exported by registry cache")
    if "cargo build --locked --release -p nyxid-machine-updater" not in text:
        failures.append("production compile must stay locked release")
    if "cargo test --locked --release -p nyxid-machine-updater --no-run" not in text:
        failures.append("release test compile was dropped")
    if 'ENTRYPOINT ["/nyxid-machine-updater"]' not in text:
        failures.append("production entrypoint changed")
    for name in (
        "dependency-recipe",
        "dependencies",
        "test-dependencies",
        "updater-sources",
        "updater-test-fixtures",
        "builder",
        "tests-build",
        "production",
        "e2e",
        "final",
    ):
        if name not in parsed:
            failures.append(f"missing stage {name}")
    if failures:
        return failures
    if parsed["tests-build"]["parent"] != "test-dependencies":
        failures.append("tests-build must be a sibling of the production compile, not FROM builder")
    if parsed["builder"]["parent"] != "dependencies":
        failures.append("builder must extend the dependency layer")
    if parsed["test-dependencies"]["parent"] != "dependencies":
        failures.append("test-dependencies must extend the dependency layer")
    if parsed["production"]["parent"] != "scratch":
        failures.append("production image must stay scratch")
    if parsed["final"]["parent"] != "production":
        failures.append("default final stage must stay the production image")
    if "updater-test-fixtures" in parsed["builder"]["from"]:
        failures.append("builder must not copy cfg(test) fixtures")
    for upstream in ("updater-sources", "dependency-recipe", "updater-test-fixtures"):
        if upstream not in parsed["tests-build"]["from"]:
            failures.append(f"tests-build must copy {upstream}")
    for info in parsed.values():
        for source in info["copies"]:
            if source in BROAD_COPIES:
                failures.append(f"broad workspace copy invalidates the compile: {source}")
    recipe = parsed["dependency-recipe"]["copies"]
    for relative in workspace_recipe_inputs():
        if not is_staged(Path(relative), recipe):
            failures.append(f"dependency recipe missing {relative}")
    for relative in (
        "cli/tests/machine_updater_fixture.mjs",
        "cli/resources/machine-container/seccomp.json",
        "cli/tests/machine_updater_driver.sh",
    ):
        if not is_staged(Path(relative), parsed["e2e"]["copies"]):
            failures.append(f"e2e runtime missing {relative}")
    return failures


def closure_failures(
    dep_infos: list[Path],
    copy_sources: list[str],
    ignore_patterns: list[tuple[bool, str]],
    label: str,
) -> tuple[list[str], set[str]]:
    failures: list[str] = []
    seen: set[str] = set()
    for dep_info in dep_infos:
        for dependency in parse_dep_info(dep_info):
            relative = repo_file(dependency)
            if relative is None:
                continue
            text = relative.as_posix()
            if text in seen:
                continue
            seen.add(text)
            if not dependency.exists():
                failures.append(f"missing {label} build dependency: {text}")
            elif not is_staged(relative, copy_sources):
                failures.append(f"not covered by {label} COPY: {text}")
            elif is_ignored(relative, ignore_patterns):
                failures.append(f"excluded by .dockerignore: {text}")
    return failures, seen


def check(
    dep_infos: list[Path],
    test_dep_infos: list[Path],
    dockerfile: Path,
    dockerignore: Path,
) -> tuple[int, set[str], set[str]]:
    text = dockerfile.read_text(encoding="utf-8")
    parsed = stages(text)
    failures = structural_failures(text, parsed)
    if any(failure.startswith("missing stage") for failure in failures):
        _report(failures)
        return 1, set(), set()
    ignore_patterns = dockerignore_patterns(dockerignore)
    production_failures, production = closure_failures(
        dep_infos, reachable_copies(parsed, PRODUCTION_STAGE), ignore_patterns, "production"
    )
    failures.extend(production_failures)
    if not production:
        failures.append("production dep-info contains no repository inputs")
    test_only: set[str] = set()
    if test_dep_infos:
        test_failures, tests = closure_failures(
            test_dep_infos, reachable_copies(parsed, TEST_STAGE), ignore_patterns, "test"
        )
        failures.extend(test_failures)
        if not tests:
            failures.append("test dep-info contains no repository inputs")
        test_only = tests - production
        production_copies = reachable_copies(parsed, PRODUCTION_STAGE)
        for relative in sorted(test_only):
            if relative.startswith("cli/tests/") and is_staged(Path(relative), production_copies):
                failures.append(f"cfg(test) fixture is in the production stage: {relative}")
    if failures:
        _report(failures)
        return 1, production, test_only
    return 0, production, test_only


def _report(failures: list[str]) -> None:
    print("Updater Docker staging does not cover rustc build inputs:", file=sys.stderr)
    for failure in failures:
        print(f"  - {failure}", file=sys.stderr)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--dep-info", type=Path, action="append", default=[])
    parser.add_argument("--test-dep-info", type=Path, action="append", default=[])
    parser.add_argument("--dockerfile", type=Path, default=DEFAULT_DOCKERFILE)
    parser.add_argument("--dockerignore", type=Path, default=DEFAULT_DOCKERIGNORE)
    args = parser.parse_args()
    dep_infos = _expand_dep_infos(args.dep_info)
    test_dep_infos = _expand_dep_infos(args.test_dep_info)
    if args.test_dep_info and not test_dep_infos:
        print("test dep-info directory contains no .d files", file=sys.stderr)
        return 1
    if not dep_infos:
        print(
            "missing production dep-info; run "
            "`cargo build --release -p nyxid-machine-updater` and pass "
            "--dep-info for the binary, library, and nyxid-machine .d files",
            file=sys.stderr,
        )
        return 1
    try:
        status, production, test_only = check(dep_infos, test_dep_infos, args.dockerfile, args.dockerignore)
    except (OSError, ValueError) as error:
        print(f"Updater Docker embed guard failed: {error}", file=sys.stderr)
        return 1
    if status != 0:
        return status
    print(
        f"Updater Docker staging covers {len(production)} production rustc inputs"
        f" and {len(test_only)} additional test inputs."
    )
    return 0


def _expand_dep_infos(paths: list[Path]) -> list[Path]:
    files: list[Path] = []
    for path in paths:
        if path.is_dir():
            files.extend(sorted(item for item in path.rglob("*.d") if item.is_file()))
        else:
            files.append(path)
    return files


if __name__ == "__main__":
    raise SystemExit(main())
