"""Deterministic tests for scripts/check-aevatar-chat-drift.py."""

from __future__ import annotations

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


NYXID_ROOT = Path(__file__).resolve().parents[2]
CHECKER = NYXID_ROOT / "scripts" / "check-aevatar-chat-drift.py"
WATCHED_PATH = "agents/Aevatar.GAgents.NyxidChat/contract.txt"
UNWATCHED_PATH = "unwatched/notes.txt"
BRANCH = "feature/integrate"


def nyxid_status() -> str:
    return subprocess.check_output(
        ["git", "status", "--short"],
        cwd=NYXID_ROOT,
        text=True,
    )


def git(repo: Path, *args: str) -> str:
    return subprocess.check_output(
        ["git", "-C", str(repo), *args],
        text=True,
    ).strip()


def write_pin(path: Path, remote: str, effective_chat_sha: str) -> None:
    payload = {
        "remote": remote,
        "branch": BRANCH,
        "remote_head": effective_chat_sha,
        "effective_chat_sha": effective_chat_sha,
        "watched_paths": [WATCHED_PATH],
        "public_commands": [],
        "internal_actions": [],
        "context_attachments": {},
        "delete": {},
        "keepalive_seconds": 15,
        "action_registry": {
            "schema_version": 4,
            "revision_is_observability_label": True,
            "unknown_or_divergent_descriptors": "degrade_per_action",
        },
    }
    path.write_text(json.dumps(payload), encoding="utf-8")


def init_repo(repo: Path) -> None:
    repo.mkdir(parents=True)
    subprocess.check_call(
        ["git", "init", "-b", BRANCH, str(repo)],
        stdout=subprocess.DEVNULL,
    )
    git(repo, "config", "user.name", "ac0-test")
    git(repo, "config", "user.email", "ac0-test@example.com")
    git(repo, "config", "commit.gpgsign", "false")


def commit_file(repo: Path, relative: str, contents: str, message: str) -> str:
    target = repo / relative
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(contents, encoding="utf-8")
    git(repo, "add", relative)
    git(repo, "commit", "-m", message)
    return git(repo, "rev-parse", "HEAD")


def run_checker(*, pin: Path, repo: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [
            sys.executable,
            "-B",
            str(CHECKER),
            "--pin",
            str(pin),
            "--remote",
            str(repo),
            "--branch",
            BRANCH,
            "--repo",
            str(repo),
            "--json",
        ],
        cwd=NYXID_ROOT,
        text=True,
        capture_output=True,
        check=False,
    )


class CheckAevatarChatDriftTests(unittest.TestCase):
    def test_clean_case_exits_zero_with_empty_path_list(self) -> None:
        with tempfile.TemporaryDirectory(prefix="ac0-clean-") as raw:
            root = Path(raw)
            repo = root / "repo"
            init_repo(repo)
            pin_sha = commit_file(repo, WATCHED_PATH, "pinned\n", "pin")
            pin_path = root / "pin.json"
            write_pin(pin_path, str(repo), pin_sha)

            before = nyxid_status()
            result = run_checker(pin=pin_path, repo=repo)
            after = nyxid_status()

            self.assertEqual(before, after)
            self.assertEqual(result.returncode, 0, result.stderr)
            receipt = json.loads(result.stdout)
            self.assertEqual(receipt["observed_head"], pin_sha)
            self.assertEqual(receipt["effective_chat_sha"], pin_sha)
            self.assertEqual(receipt["changed_watched_paths"], [])
            self.assertRegex(receipt["fetch_timestamp"], r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$")

    def test_drift_case_exits_one_listing_only_the_watched_path(self) -> None:
        with tempfile.TemporaryDirectory(prefix="ac0-drift-") as raw:
            root = Path(raw)
            repo = root / "repo"
            init_repo(repo)
            pin_sha = commit_file(repo, WATCHED_PATH, "pinned\n", "pin")
            commit_file(repo, WATCHED_PATH, "moved\n", "watched drift")
            commit_file(repo, UNWATCHED_PATH, "noise\n", "unwatched")
            head = git(repo, "rev-parse", "HEAD")
            pin_path = root / "pin.json"
            write_pin(pin_path, str(repo), pin_sha)

            before = nyxid_status()
            result = run_checker(pin=pin_path, repo=repo)
            after = nyxid_status()

            self.assertEqual(before, after)
            self.assertEqual(result.returncode, 1, result.stderr)
            receipt = json.loads(result.stdout)
            self.assertEqual(receipt["observed_head"], head)
            self.assertEqual(receipt["effective_chat_sha"], pin_sha)
            self.assertEqual(receipt["changed_watched_paths"], [WATCHED_PATH])
            self.assertNotIn(UNWATCHED_PATH, receipt["changed_watched_paths"])


if __name__ == "__main__":
    unittest.main()
