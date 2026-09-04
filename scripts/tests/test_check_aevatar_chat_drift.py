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
WATCHED_DIRECTORY = "agents/Aevatar.GAgents.NyxidChat/"
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
        stderr=subprocess.DEVNULL,
    ).strip()


def write_pin(
    path: Path,
    remote: str,
    effective_chat_sha: str,
    *,
    remote_head: str | None = None,
    watched_paths: tuple[str, ...] = (WATCHED_PATH,),
) -> None:
    payload = {
        "remote": remote,
        "branch": BRANCH,
        "remote_head": remote_head or effective_chat_sha,
        "effective_chat_sha": effective_chat_sha,
        "watched_paths": list(watched_paths),
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


def run_checker(
    *, pin: Path, repo: Path, branch: str = BRANCH
) -> subprocess.CompletedProcess[str]:
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
            branch,
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
    def assert_tool_error(
        self,
        result: subprocess.CompletedProcess[str],
        expected_message: str,
    ) -> None:
        combined = result.stdout + result.stderr
        self.assertEqual(result.returncode, 2, combined)
        self.assertIn(expected_message, combined)
        self.assertNotIn("Traceback", combined)

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
            self.assertEqual(receipt["pinned_remote_head"], pin_sha)
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
            self.assertEqual(receipt["pinned_remote_head"], pin_sha)
            self.assertEqual(receipt["effective_chat_sha"], pin_sha)
            self.assertEqual(receipt["changed_watched_paths"], [WATCHED_PATH])
            self.assertNotIn(UNWATCHED_PATH, receipt["changed_watched_paths"])
            self.assertIn("differs from observed_head", result.stderr)

    def test_receipt_carries_pinned_and_observed_heads(self) -> None:
        with tempfile.TemporaryDirectory(prefix="ac0-heads-") as raw:
            root = Path(raw)
            repo = root / "repo"
            init_repo(repo)
            pin_sha = commit_file(repo, WATCHED_PATH, "pinned\n", "pin")
            commit_file(repo, UNWATCHED_PATH, "noise\n", "unwatched")
            head = git(repo, "rev-parse", "HEAD")
            pin_path = root / "pin.json"
            write_pin(pin_path, str(repo), pin_sha, remote_head=pin_sha)

            result = run_checker(pin=pin_path, repo=repo)
            self.assertEqual(result.returncode, 0, result.stderr)
            receipt = json.loads(result.stdout)
            self.assertEqual(receipt["pinned_remote_head"], pin_sha)
            self.assertEqual(receipt["observed_head"], head)
            self.assertNotEqual(receipt["pinned_remote_head"], receipt["observed_head"])
            self.assertEqual(receipt["changed_watched_paths"], [])
            self.assertIn("differs from observed_head", result.stderr)

    def test_non_ancestor_pin_exits_two(self) -> None:
        with tempfile.TemporaryDirectory(prefix="ac0-orphan-") as raw:
            root = Path(raw)
            repo = root / "repo"
            init_repo(repo)
            pin_sha = commit_file(repo, WATCHED_PATH, "pinned\n", "pin")
            git(repo, "checkout", "--orphan", "unrelated")
            git(repo, "commit", "--allow-empty", "-m", "unrelated root")
            unrelated_head = git(repo, "rev-parse", "HEAD")
            git(repo, "branch", "-f", BRANCH, unrelated_head)
            git(repo, "checkout", BRANCH)
            pin_path = root / "pin.json"
            write_pin(pin_path, str(repo), pin_sha)

            before = nyxid_status()
            result = run_checker(pin=pin_path, repo=repo)
            after = nyxid_status()

            self.assertEqual(before, after)
            self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
            combined = result.stdout + result.stderr
            self.assertIn(pin_sha, combined)
            self.assertIn(unrelated_head, combined)
            self.assertIn("not an ancestor", combined)

    def test_non_hex_sha_fields_exit_two(self) -> None:
        with tempfile.TemporaryDirectory(prefix="ac0-non-hex-sha-") as raw:
            root = Path(raw)
            repo = root / "repo"
            init_repo(repo)
            pin_sha = commit_file(repo, WATCHED_PATH, "pinned\n", "pin")

            cases = (
                ("remote_head", pin_sha, "g" * 40),
                ("effective_chat_sha", "g" * 40, pin_sha),
            )
            for field, effective_chat_sha, remote_head in cases:
                with self.subTest(field=field):
                    pin_path = root / f"{field}.json"
                    write_pin(
                        pin_path,
                        str(repo),
                        effective_chat_sha,
                        remote_head=remote_head,
                    )

                    result = run_checker(pin=pin_path, repo=repo)

                    self.assert_tool_error(
                        result,
                        f"pin.{field} must be exactly 40 lowercase hexadecimal characters",
                    )

    def test_symbolic_sha_exits_two(self) -> None:
        with tempfile.TemporaryDirectory(prefix="ac0-symbolic-sha-") as raw:
            root = Path(raw)
            repo = root / "repo"
            init_repo(repo)
            pin_sha = commit_file(repo, WATCHED_PATH, "pinned\n", "pin")
            pin_path = root / "pin.json"
            write_pin(pin_path, str(repo), "FETCH_HEAD", remote_head=pin_sha)

            result = run_checker(pin=pin_path, repo=repo)

            self.assert_tool_error(
                result,
                "pin.effective_chat_sha must be exactly 40 lowercase hexadecimal characters",
            )

    def test_pathspec_exclusion_exits_two(self) -> None:
        with tempfile.TemporaryDirectory(prefix="ac0-pathspec-") as raw:
            root = Path(raw)
            repo = root / "repo"
            init_repo(repo)
            pin_sha = commit_file(repo, WATCHED_PATH, "pinned\n", "pin")
            commit_file(repo, WATCHED_PATH, "moved\n", "watched drift")
            pin_path = root / "pin.json"
            write_pin(
                pin_path,
                str(repo),
                pin_sha,
                watched_paths=(f":(exclude){WATCHED_PATH}",),
            )

            result = run_checker(pin=pin_path, repo=repo)

            self.assert_tool_error(result, "must not start with ':'")

    def test_parent_path_segment_exits_two(self) -> None:
        with tempfile.TemporaryDirectory(prefix="ac0-parent-path-") as raw:
            root = Path(raw)
            repo = root / "repo"
            init_repo(repo)
            pin_sha = commit_file(repo, WATCHED_PATH, "pinned\n", "pin")
            pin_path = root / "pin.json"
            write_pin(
                pin_path,
                str(repo),
                pin_sha,
                watched_paths=("agents/../contract.txt",),
            )

            result = run_checker(pin=pin_path, repo=repo)

            self.assert_tool_error(result, "must not contain '..' segments")

    def test_invalid_watched_path_shapes_exit_two(self) -> None:
        with tempfile.TemporaryDirectory(prefix="ac0-path-shapes-") as raw:
            root = Path(raw)
            repo = root / "repo"
            init_repo(repo)
            pin_sha = commit_file(repo, WATCHED_PATH, "pinned\n", "pin")

            cases = (
                ("leading-slash", "/agents/contract.txt", "must not start with '/'"),
                ("empty-segment", "agents//contract.txt", "must not contain empty segments"),
            )
            for name, watched_path, expected_message in cases:
                with self.subTest(name=name):
                    pin_path = root / f"{name}.json"
                    write_pin(
                        pin_path,
                        str(repo),
                        pin_sha,
                        watched_paths=(watched_path,),
                    )

                    result = run_checker(pin=pin_path, repo=repo)

                    self.assert_tool_error(result, expected_message)

    def test_nul_watched_path_exits_two(self) -> None:
        with tempfile.TemporaryDirectory(prefix="ac0-nul-path-") as raw:
            root = Path(raw)
            repo = root / "repo"
            init_repo(repo)
            pin_sha = commit_file(repo, WATCHED_PATH, "pinned\n", "pin")
            pin_path = root / "pin.json"
            write_pin(
                pin_path,
                str(repo),
                pin_sha,
                watched_paths=("agents/contract\0.txt",),
            )

            result = run_checker(pin=pin_path, repo=repo)

            self.assert_tool_error(result, "contains unsupported characters")

    def test_invalid_utf8_pin_exits_two(self) -> None:
        with tempfile.TemporaryDirectory(prefix="ac0-invalid-utf8-") as raw:
            root = Path(raw)
            repo = root / "repo"
            init_repo(repo)
            pin_path = root / "pin.json"
            pin_path.write_bytes(b"\xff")

            result = run_checker(pin=pin_path, repo=repo)

            self.assert_tool_error(result, "pin is not valid UTF-8")

    def test_branch_refspec_injection_exits_two_without_creating_ref(self) -> None:
        with tempfile.TemporaryDirectory(prefix="ac0-branch-refspec-") as raw:
            root = Path(raw)
            repo = root / "repo"
            init_repo(repo)
            pin_sha = commit_file(repo, WATCHED_PATH, "pinned\n", "pin")
            pin_path = root / "pin.json"
            write_pin(pin_path, str(repo), pin_sha)
            injected_ref = "refs/heads/ac0-injected"
            injected_branch = f"{BRANCH}:{injected_ref}"

            before = subprocess.run(
                ["git", "-C", str(repo), "show-ref", "--verify", injected_ref],
                capture_output=True,
                text=True,
                check=False,
            )
            result = run_checker(pin=pin_path, repo=repo, branch=injected_branch)
            after = subprocess.run(
                ["git", "-C", str(repo), "show-ref", "--verify", injected_ref],
                capture_output=True,
                text=True,
                check=False,
            )

            self.assertNotEqual(before.returncode, 0)
            self.assertNotEqual(after.returncode, 0)
            self.assert_tool_error(result, "branch is not a valid Git branch name")

    def test_trailing_slash_directory_path_is_valid(self) -> None:
        with tempfile.TemporaryDirectory(prefix="ac0-directory-path-") as raw:
            root = Path(raw)
            repo = root / "repo"
            init_repo(repo)
            pin_sha = commit_file(repo, WATCHED_PATH, "pinned\n", "pin")
            pin_path = root / "pin.json"
            write_pin(
                pin_path,
                str(repo),
                pin_sha,
                watched_paths=(WATCHED_DIRECTORY,),
            )

            result = run_checker(pin=pin_path, repo=repo)

            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()
