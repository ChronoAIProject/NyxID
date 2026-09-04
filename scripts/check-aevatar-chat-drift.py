#!/usr/bin/env python3
"""Compare Aevatar watched chat paths from a pinned SHA to the live branch head.

Receipt fields are pinned_remote_head, observed_head, effective_chat_sha,
fetch_timestamp, and changed_watched_paths. Exit 0 when the watched-path
list is empty, 1 when it is not, and 2 on tool or ancestry failure. A moved
remote head with no watched-path change is still clean.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path


EXIT_CLEAN = 0
EXIT_DRIFT = 1
EXIT_TOOL = 2
GIT_TIMEOUT_SECS = 180


class ToolError(Exception):
    pass


@dataclass(frozen=True)
class ContractPin:
    remote: str
    branch: str
    remote_head: str
    effective_chat_sha: str
    watched_paths: tuple[str, ...]


@dataclass(frozen=True)
class DriftReceipt:
    effective_chat_sha: str
    pinned_remote_head: str
    observed_head: str
    fetch_timestamp: str
    changed_watched_paths: tuple[str, ...]

    def to_json(self) -> dict[str, object]:
        return {
            "effective_chat_sha": self.effective_chat_sha,
            "pinned_remote_head": self.pinned_remote_head,
            "observed_head": self.observed_head,
            "fetch_timestamp": self.fetch_timestamp,
            "changed_watched_paths": list(self.changed_watched_paths),
        }


def load_pin(path: Path) -> ContractPin:
    try:
        raw = json.loads(path.read_text(encoding="utf-8"))
    except OSError as error:
        raise ToolError(f"cannot read pin {path}: {error}") from error
    except json.JSONDecodeError as error:
        raise ToolError(f"pin is not valid JSON: {error}") from error
    if not isinstance(raw, dict):
        raise ToolError("pin must be a JSON object")

    remote = _require_str(raw, "remote")
    branch = _require_str(raw, "branch")
    remote_head = _require_str(raw, "remote_head")
    effective_chat_sha = _require_str(raw, "effective_chat_sha")
    watched = raw.get("watched_paths")
    if not isinstance(watched, list) or not watched:
        raise ToolError("pin.watched_paths must be a nonempty array")
    paths: list[str] = []
    for item in watched:
        if not isinstance(item, str) or not item.strip():
            raise ToolError("pin.watched_paths entries must be nonempty strings")
        paths.append(item)
    return ContractPin(
        remote=remote,
        branch=branch,
        remote_head=remote_head,
        effective_chat_sha=effective_chat_sha,
        watched_paths=tuple(paths),
    )


def _require_str(raw: dict[str, object], key: str) -> str:
    value = raw.get(key)
    if not isinstance(value, str) or not value.strip():
        raise ToolError(f"pin.{key} must be a nonempty string")
    return value


def _git_env() -> dict[str, str]:
    env = os.environ.copy()
    env["GIT_TERMINAL_PROMPT"] = "0"
    return env


def git(repo: str, *args: str) -> subprocess.CompletedProcess[str]:
    try:
        return subprocess.run(
            ["git", "-C", repo, *args],
            capture_output=True,
            text=True,
            env=_git_env(),
            timeout=GIT_TIMEOUT_SECS,
            check=False,
        )
    except FileNotFoundError as error:
        raise ToolError("git is not installed") from error
    except subprocess.TimeoutExpired as error:
        raise ToolError(f"git timed out: {' '.join(args)}") from error


def git_ok(repo: str, *args: str) -> str:
    result = git(repo, *args)
    if result.returncode != 0:
        detail = (result.stderr or result.stdout or "").strip()
        raise ToolError(detail or f"git {' '.join(args)} failed")
    return result.stdout.strip()


def utc_now() -> str:
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def ensure_git_repo(path: str) -> None:
    result = git(path, "rev-parse", "--git-dir")
    if result.returncode != 0:
        raise ToolError(f"{path} is not a git repository")


def fetch_branch(repo: str, remote: str, branch: str, pin_sha: str) -> str:
    fetched = git(repo, "fetch", "--", remote, f"refs/heads/{branch}")
    if fetched.returncode != 0:
        detail = (fetched.stderr or fetched.stdout or "").strip()
        raise ToolError(detail or f"failed to fetch {remote} {branch}")
    observed_head = git_ok(repo, "rev-parse", "--verify", "FETCH_HEAD")
    present = git(repo, "cat-file", "-e", f"{pin_sha}^{{commit}}")
    if present.returncode != 0:
        pin_fetch = git(repo, "fetch", "--", remote, pin_sha)
        if pin_fetch.returncode != 0:
            detail = (pin_fetch.stderr or pin_fetch.stdout or "").strip()
            raise ToolError(detail or f"failed to fetch pin SHA {pin_sha}")
        still_missing = git(repo, "cat-file", "-e", f"{pin_sha}^{{commit}}")
        if still_missing.returncode != 0:
            raise ToolError(f"pin SHA {pin_sha} is not in {remote}")
    ancestry = git(repo, "merge-base", "--is-ancestor", pin_sha, observed_head)
    if ancestry.returncode == 1:
        raise ToolError(
            f"effective_chat_sha {pin_sha} is not an ancestor of observed_head {observed_head}"
        )
    if ancestry.returncode != 0:
        detail = (ancestry.stderr or ancestry.stdout or "").strip()
        raise ToolError(
            detail
            or f"failed to verify ancestry of {pin_sha} against {observed_head}"
        )
    return observed_head


def changed_paths(repo: str, pin_sha: str, observed_head: str, watched: tuple[str, ...]) -> tuple[str, ...]:
    output = git_ok(
        repo,
        "diff",
        "--name-only",
        "--no-renames",
        pin_sha,
        observed_head,
        "--",
        *watched,
    )
    paths = [line for line in output.splitlines() if line]
    return tuple(paths)


def run_check(pin: ContractPin, remote: str, branch: str, reuse_repo: str | None) -> DriftReceipt:
    if reuse_repo is not None:
        ensure_git_repo(reuse_repo)
        observed_head = fetch_branch(reuse_repo, remote, branch, pin.effective_chat_sha)
        fetch_timestamp = utc_now()
        changed = changed_paths(
            reuse_repo,
            pin.effective_chat_sha,
            observed_head,
            pin.watched_paths,
        )
        return DriftReceipt(
            effective_chat_sha=pin.effective_chat_sha,
            pinned_remote_head=pin.remote_head,
            observed_head=observed_head,
            fetch_timestamp=fetch_timestamp,
            changed_watched_paths=changed,
        )

    with tempfile.TemporaryDirectory(prefix="aevatar-chat-drift-") as tmp:
        init = subprocess.run(
            ["git", "init", "--bare", tmp],
            capture_output=True,
            text=True,
            env=_git_env(),
            timeout=GIT_TIMEOUT_SECS,
            check=False,
        )
        if init.returncode != 0:
            detail = (init.stderr or init.stdout or "").strip()
            raise ToolError(detail or "failed to create temporary clone")
        observed_head = fetch_branch(tmp, remote, branch, pin.effective_chat_sha)
        fetch_timestamp = utc_now()
        changed = changed_paths(
            tmp,
            pin.effective_chat_sha,
            observed_head,
            pin.watched_paths,
        )
        return DriftReceipt(
            effective_chat_sha=pin.effective_chat_sha,
            pinned_remote_head=pin.remote_head,
            observed_head=observed_head,
            fetch_timestamp=fetch_timestamp,
            changed_watched_paths=changed,
        )


def emit(receipt: DriftReceipt, as_json: bool) -> None:
    if as_json:
        json.dump(receipt.to_json(), sys.stdout, indent=2)
        sys.stdout.write("\n")
        return
    sys.stdout.write(f"pinned_remote_head: {receipt.pinned_remote_head}\n")
    sys.stdout.write(f"observed_head: {receipt.observed_head}\n")
    sys.stdout.write(f"fetch_timestamp: {receipt.fetch_timestamp}\n")
    sys.stdout.write("changed_watched_paths:\n")
    if not receipt.changed_watched_paths:
        sys.stdout.write("(none)\n")
        return
    for path in receipt.changed_watched_paths:
        sys.stdout.write(f"{path}\n")


def emit_error(message: str, as_json: bool) -> None:
    if as_json:
        json.dump({"error": message}, sys.stdout, indent=2)
        sys.stdout.write("\n")
        return
    sys.stderr.write(f"{message}\n")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Fail when Aevatar watched chat paths move after the pinned SHA.",
    )
    parser.add_argument("--pin", required=True, help="Path to aevatar-chat-contract-pin.json")
    parser.add_argument("--remote", help="Git remote URL. Defaults to the pin remote.")
    parser.add_argument("--branch", help="Branch name. Defaults to the pin branch.")
    parser.add_argument(
        "--repo",
        help="Existing clone to reuse. Only git fetch runs against it.",
    )
    parser.add_argument("--json", action="store_true", help="Write a machine-readable receipt.")
    args = parser.parse_args(argv)

    try:
        pin = load_pin(Path(args.pin))
        remote = args.remote.strip() if args.remote else pin.remote
        branch = args.branch.strip() if args.branch else pin.branch
        if not remote or not branch:
            raise ToolError("remote and branch are required")
        reuse = args.repo
        receipt = run_check(pin, remote, branch, reuse)
    except ToolError as error:
        emit_error(str(error), args.json)
        return EXIT_TOOL
    except OSError as error:
        emit_error(str(error), args.json)
        return EXIT_TOOL

    emit(receipt, args.json)
    if receipt.pinned_remote_head != receipt.observed_head:
        sys.stderr.write(
            "notice: pinned_remote_head "
            f"{receipt.pinned_remote_head} differs from observed_head "
            f"{receipt.observed_head}\n"
        )
    if receipt.changed_watched_paths:
        return EXIT_DRIFT
    return EXIT_CLEAN


if __name__ == "__main__":
    sys.exit(main())
