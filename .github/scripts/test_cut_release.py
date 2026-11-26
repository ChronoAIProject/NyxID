import json
import os
import subprocess
import tempfile
import unittest
from datetime import datetime, timezone
from pathlib import Path


SCRIPT = Path("scripts/cut-release.sh").resolve()
SHA = "abc123456789" * 3 + "abcd"
MAIN = "b" * 40
OLD_PR = "https://github.com/o/r/pull/7"
NEW_PR = "https://github.com/o/r/pull/8"

# Match the actual argv. Unknown operations (including any write during a
# dry run) fail the test even if the script was already expected to refuse.
FAKE_COMMAND = """#!/usr/bin/env python3
import json, os, sys
from pathlib import Path
argv = [Path(sys.argv[0]).name, *sys.argv[1:]]
with open("calls", "a") as log:
    print(json.dumps(argv), file=log)
for command, output, status in json.loads(Path("replies").read_text()):
    if command == argv:
        if argv[:3] == ["gh", "pr", "create"]:
            Path("pr-body").write_text(sys.stdin.read())
        print(output)
        sys.exit(status)
sys.exit("Unexpected command: " + repr(argv))
"""


class CutReleaseTests(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        for name in ("git", "curl", "gh"):
            command = self.root / name
            command.write_text(FAKE_COMMAND)
            command.chmod(0o755)
        self.day = datetime.now(timezone.utc).date().isoformat()
        self.branch = f"release/{self.day}"
        self.replies = {}
        self.api = "repos/o/r"
        self.reply(["git", "remote", "get-url", "origin"], "git@github.com:o/r.git")
        self.staging("backend", SHA[:12])
        self.staging("frontend", SHA[:12])
        self.resolve(SHA[:12])
        self.dev_compare = ["gh", "api", f"{self.api}/compare/dev...{SHA}?per_page=1", "--jq", ".status"]
        self.reply(self.dev_compare, "behind")
        self.reply(["gh", "api", f"{self.api}/git/ref/heads/main", "--jq", ".object.sha"], MAIN)
        self.comparison = ["gh", "api", "--paginate", "--slurp",
                           f"{self.api}/compare/{MAIN}...{SHA}?per_page=100"]
        released = "a" * 40
        first = {"sha": "1" * 40, "parents": [{"sha": released}],
                 "commit": {"message": "fix: first change (#5)"}}
        side = {"sha": "2" * 40, "parents": [{"sha": released}],
                "commit": {"message": "side branch commit"}}
        snapshot = {"sha": SHA, "parents": [{"sha": first["sha"]}, {"sha": side["sha"]}],
                    "commit": {"message": "Merge pull request #6 from o/topic\n\nFeature title"}}
        self.reply(self.comparison, json.dumps([
            {"status": "diverged", "total_commits": 3, "commits": [first, side]},
            {"status": "diverged", "total_commits": 3, "commits": [snapshot]},
        ]))
        self.reverse = ["gh", "api", "--paginate", "--slurp",
                        f"{self.api}/compare/{SHA}...{MAIN}?per_page=100"]
        self.reply(self.reverse, json.dumps([{"status": "diverged", "total_commits": 1, "commits": [
            {"sha": MAIN, "parents": [{"sha": "9" * 40}, {"sha": released}]},
        ]}]))
        self.main_pulls = ["gh", "api", "--paginate", "--slurp",
                           f"{self.api}/commits/{MAIN}/pulls?per_page=100"]
        self.main_pr = {"base": {"ref": "main", "repo": {"id": 1}},
                        "head": {"ref": "release/2026-11-18", "repo": {"id": 1}},
                        "merged_at": "2026-11-18", "merge_commit_sha": MAIN}
        self.reply(self.main_pulls, json.dumps([[self.main_pr]]))
        self.refs = ["gh", "api", "--paginate", f"{self.api}/git/matching-refs/heads/{self.branch}",
                     "--jq", ".[].ref"]
        self.reply(self.refs, "")
        self.pulls = ["gh", "api", "--paginate", f"{self.api}/pulls?state=open&base=main&per_page=100",
                      "--jq", '\n  .[] | select(.head.repo.id == .base.repo.id)\n'
                      '      | select(.head.ref == "dev" or (.head.ref | startswith("release/"))) | .html_url']
        self.reply(self.pulls, "")

    def reply(self, command, output, status=0):
        self.replies[tuple(command)] = (output, status)

    def staging(self, service, sha):
        url = {"backend": "https://staging-api.nyxid.ai/health",
               "frontend": "https://staging.nyxid.ai/build-version.json"}[service]
        self.reply(["curl", "--fail", "--silent", "--show-error", "--max-time", "20",
                    "-H", "Cache-Control: no-cache", url], json.dumps({"commit": sha}))

    def resolve(self, prefix):
        self.reply(["gh", "api", f"{self.api}/commits/{prefix}", "--jq", ".sha"], SHA)

    def cut(self, *args, success=True):
        (self.root / "replies").write_text(json.dumps([
            [cmd, *reply] for cmd, reply in self.replies.items()
        ]))
        (self.root / "calls").write_text("")
        result = subprocess.run([str(SCRIPT), *args], cwd=self.root, capture_output=True, text=True,
                                env={"PATH": str(self.root) + os.pathsep + os.environ["PATH"]})
        self.calls = [json.loads(line) for line in (self.root / "calls").read_text().splitlines()]
        for cmd in self.calls:
            self.assertIn(tuple(cmd), self.replies)
        self.assertEqual(result.returncode == 0, success, result.stdout + result.stderr)
        return result.stdout + result.stderr

    def test_staging_cut_lists_only_first_parent_prs_across_pages(self):
        output = self.cut("--dry-run")
        self.assertIn(f"Create {self.branch} at {SHA} in o/r", output)
        self.assertIn(f"release: {self.day} ({SHA[:7]})", output)
        self.assertIn("- fix: first change (#5) (1111111)\n- Feature title (#6) (abc1234)", output)
        self.assertNotIn("side branch commit", output)

    def test_cut_after_main_sync_lists_dev_first_parents_in_order(self):
        common, feature, sync = "1" * 40, "2" * 40, "3" * 40
        # main contains the production hotfix. The sync brings that entire
        # history in on parent two, between two unreleased dev changes.
        rows = [
            {"sha": feature, "parents": [{"sha": common}], "commit": {"message": "Feature (#10)"}},
            {"sha": sync, "parents": [{"sha": feature}, {"sha": MAIN}],
             "commit": {"message": "Merge pull request #11 from o/main\n\nSync production hotfix"}},
            {"sha": SHA, "parents": [{"sha": sync}], "commit": {"message": "Follow-up (#12)"}},
        ]
        self.reply(self.comparison, json.dumps([{"status": "ahead", "total_commits": 3, "commits": rows}]))
        output = self.cut("--dry-run")
        self.assertEqual([line for line in output.splitlines() if line.startswith("- ")], [
            "- Feature (#10) (2222222)", "- Sync production hotfix (#11) (3333333)",
            "- Follow-up (#12) (abc1234)",
        ])

    def test_cut_refuses_unsynced_hotfix_before_writes(self):
        hotfix = "4" * 40
        self.reply(self.reverse, json.dumps([
            {"status": "diverged", "total_commits": 2, "commits": [
                {"sha": hotfix, "parents": [{"sha": "a" * 40}]},
            ]},
            {"status": "diverged", "total_commits": 2, "commits": [
                {"sha": MAIN, "parents": [{"sha": "a" * 40}, {"sha": hotfix}]},
            ]},
        ]))
        self.assertIn("Merge main into dev first", self.cut(success=False))
        self.assertEqual(self.calls[-1], self.reverse)

    def test_hotfix_branch_already_on_dev_still_requires_its_main_merge(self):
        pr = dict(self.main_pr, head={"ref": "hotfix/fix", "repo": {"id": 1}})
        self.reply(self.main_pulls, json.dumps([[pr]]))
        self.assertIn("Merge main into dev first", self.cut("--dry-run", success=False))

    def test_staging_disagreement_identifies_both_services(self):
        self.staging("frontend", "d" * 12)
        output = self.cut("--dry-run", success=False)
        self.assertIn(f"Backend {SHA[:12]}, frontend {'d' * 12}", output)
        self.assertFalse(any(cmd[0] == "gh" for cmd in self.calls))

    def test_edited_commit_subject_uses_merged_dev_pr_title_and_number(self):
        self.reply(self.comparison, json.dumps([{"status": "ahead", "total_commits": 1, "commits": [
            {"sha": SHA, "parents": [{"sha": MAIN}], "commit": {"message": "edited subject"}},
        ]}]))
        pr = {"base": {"ref": "dev"}, "merge_commit_sha": SHA, "merged_at": "2026-11-18",
              "title": "Original PR title", "number": 5}
        self.reply(["gh", "api", "--paginate", "--slurp", f"{self.api}/commits/{SHA}/pulls?per_page=100"],
                   json.dumps([[pr, dict(pr, base={"ref": "main"}, number=99)]]))
        output = self.cut("--dry-run")
        self.assertIn("- Original PR title (#5)", output)
        self.assertNotIn("#99", output)

    def test_override_bypasses_staging_and_skips_taken_names(self):
        self.resolve(SHA[:7].upper())
        self.reply(self.refs, f"refs/heads/{self.branch}\nrefs/heads/{self.branch}-2")
        output = self.cut("--commit", SHA[:7].upper(), "--dry-run")
        self.assertIn(f"Create {self.branch}-3 at {SHA}", output)
        self.assertFalse(any(cmd[0] == "curl" for cmd in self.calls))

    def test_refuses_commit_outside_dev_or_already_on_main(self):
        self.reply(self.dev_compare, "diverged")
        self.assertIn("outside origin/dev", self.cut("--dry-run", success=False))
        self.reply(self.dev_compare, "identical")
        for status in ("identical", "behind"):
            with self.subTest(status=status):
                self.reply(self.comparison, json.dumps([{"status": status, "commits": []}]))
                self.assertIn("already contained in main", self.cut("--dry-run", success=False))

    def test_open_release_requires_supersede_and_previews_close(self):
        self.reply(self.pulls, OLD_PR)
        self.assertIn(OLD_PR, self.cut("--dry-run", success=False))
        output = self.cut("--dry-run", "--supersede")
        self.assertIn("After opening the new PR", output)
        self.assertIn(OLD_PR, output)

    def test_supersede_closes_only_after_successful_creation(self):
        self.reply(self.pulls, OLD_PR)
        create_ref = ["gh", "api", "--method", "POST", f"{self.api}/git/refs", "-f",
                      f"ref=refs/heads/{self.branch}", "-f", f"sha={SHA}", "--silent"]
        create_pr = ["gh", "pr", "create", "--repo", "o/r", "--base", "main", "--head",
                     self.branch, "--title", f"release: {self.day} ({SHA[:7]})", "--body-file", "-"]
        close = ["gh", "pr", "close", OLD_PR, "--comment", f"Superseded by {NEW_PR}."]
        self.reply(create_ref, "")
        self.reply(create_pr, "", 1)
        self.cut("--supersede", success=False)
        self.assertEqual(self.calls[-2:], [create_ref, create_pr])
        self.reply(create_pr, NEW_PR)
        self.reply(close, "")
        output = self.cut("--supersede")
        self.assertIn(NEW_PR, output)
        self.assertEqual(self.calls[-3:], [create_ref, create_pr, close])
        self.assertIn((self.root / "pr-body").read_text(), output)
