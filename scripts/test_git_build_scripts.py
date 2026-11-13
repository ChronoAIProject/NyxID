"""Run the actual build scripts with plain Cargo in disposable Git layouts."""

import os
from pathlib import Path
import shlex
import shutil
import subprocess
import tempfile
import unittest

from rust_fixture import requires_rust, rust_command, rust_environment

ROOT = Path(__file__).resolve().parent.parent


class GitBuildScriptsTests(unittest.TestCase):
    @requires_rust(tools=("git",))
    def setUp(self):
        pass

    def fixture(self, package, old_git=False):
        temporary = tempfile.TemporaryDirectory(prefix="git build fixture ")
        self.addCleanup(temporary.cleanup)
        directory = Path(temporary.name)
        repo = directory / "repo"
        source = repo / package
        (source / "src").mkdir(parents=True)
        (source / "Cargo.toml").write_text(
            '[package]\nname="stamp-fixture"\nversion="0.1.0"\nedition="2024"\n[workspace]\n'
        )
        (source / "Cargo.lock").write_text(
            'version = 4\n[[package]]\nname = "stamp-fixture"\nversion = "0.1.0"\n'
        )
        shutil.copyfile(ROOT / package / "build.rs", source / "build.rs")
        (source / "src/main.rs").write_text('fn main() { println!("{}", env!("NYXID_GIT_HASH")); }\n')
        (repo / "README").write_text("first\n")
        env = {k: v for k, v in rust_environment().items()
               if not k.startswith("GIT_") and k not in (
                   "NYXID_GIT_HASH", "CARGO_ENCODED_RUSTFLAGS",
                   "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER", "DEVELOPER_DIR", "TOOLCHAINS", "SDKROOT")}
        env.update(CARGO_TARGET_DIR=str(directory / "target"), RUSTFLAGS="-Dwarnings",
                   RUSTUP_TOOLCHAIN="1.98.1", CARGO_TERM_COLOR="never")
        if old_git:
            shim_dir = directory / "bin"
            shim_dir.mkdir()
            shim = shim_dir / "git"
            shim.write_text(
                '#!/bin/sh\nfor arg do\ncase "$arg" in --path-format*) exit 129;; esac\ndone\n'
                f'exec {shlex.quote(shutil.which("git"))} "$@"\n'
            )
            shim.chmod(0o755)
            env["PATH"] = str(shim_dir) + os.pathsep + env["PATH"]

        def git(*args, cwd=repo):
            return subprocess.run(["git", *args], cwd=cwd, env=env, check=True,
                                  capture_output=True, text=True).stdout.strip()

        def build(cwd=repo, extra=None):
            result = subprocess.run(
                rust_command("cargo", "run", "--locked", "--manifest-path", str(cwd / package / "Cargo.toml")),
                cwd=cwd, env={**env, "CARGO_TARGET_DIR": str(directory / (cwd.name + "-target")),
                              **(extra or {})}, capture_output=True, text=True, timeout=120,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            return result.stdout.strip(), "Compiling stamp-fixture" in result.stderr

        return directory, repo, git, build

    def initialize(self, git, reflog=True):
        git("init", "-q")
        git("symbolic-ref", "HEAD", "refs/heads/main")
        git("config", "core.logAllRefUpdates", str(reflog).lower())
        git("config", "user.name", "Fixture")
        git("config", "user.email", "fixture@example.invalid")
        git("add", ".")
        git("commit", "-qm", "first")
        return git("rev-parse", "--short=12", "HEAD")

    def test_plain_cargo_head_changes_and_worktree_isolation(self):
        for package in ("backend", "cli"):
            with self.subTest(package=package):
                directory, repo, git, build = self.fixture(package)
                first = self.initialize(git)
                self.assertEqual(build(), (first, True))
                self.assertEqual(build(), (first, False))
                # Dirty state is sampled on a build-script rerun, not each invocation.
                (repo / "README").write_text("second\n")
                self.assertEqual(build(), (first, False))
                git("commit", "-qam", "second")
                second = git("rev-parse", "--short=12", "HEAD")
                self.assertEqual(build(), (second, True))
                self.assertEqual(build(), (second, False))
                git("checkout", "--detach", "HEAD~1")
                self.assertEqual(build(), (first, True))
                self.assertEqual(build(), (first, False))
                git("checkout", "main")
                self.assertEqual(build(), (second, True))
                git("pack-refs", "--all", "--prune")
                self.assertEqual(build(), (second, False))
                (repo / "README").write_text("third\n")
                git("commit", "-qam", "third")
                third = git("rev-parse", "--short=12", "HEAD")
                self.assertEqual(build(), (third, True))
                git("reset", "--hard", "HEAD~1")
                self.assertEqual(build(), (second, True))
                linked = directory / "linked"
                git("worktree", "add", "-b", "side", str(linked))
                self.assertEqual(build(cwd=linked), (second, True))
                self.assertEqual(build(cwd=linked), (second, False))
                (repo / "README").write_text("main moved\n")
                git("commit", "-qam", "main moved")
                git("pack-refs", "--all", "--prune")
                self.assertEqual(build(cwd=linked), (second, False))
                (linked / "SIDE").write_text("side moved\n")
                git("add", "SIDE", cwd=linked)
                git("commit", "-qam", "side moved", cwd=linked)
                side = git("rev-parse", "--short=12", "HEAD", cwd=linked)
                self.assertEqual(build(cwd=linked), (side, True))
                self.assertEqual(build(cwd=linked), (side, False))
                # Refresh the main target for its own checkout; then update only side.
                main = git("rev-parse", "--short=12", "HEAD")
                self.assertEqual(build(), (main, True))
                (linked / "SIDE").write_text("side again\n")
                git("commit", "-qam", "side again", cwd=linked)
                self.assertEqual(build(), (main, False))
                git("fetch", str(linked), "side:refs/remotes/fixture/side")
                self.assertEqual(build(), (main, False))
                git("rebase", "main", cwd=linked)
                rebased = git("rev-parse", "--short=12", "HEAD", cwd=linked)
                self.assertNotEqual(rebased, side)
                self.assertEqual(build(cwd=linked), (rebased, True))
                self.assertEqual(build(), (main, False))
                git("pull", "--ff-only", str(linked), "side")
                self.assertEqual(build(), (rebased, True))
                self.assertEqual(build(), (rebased, False))

    def test_packed_branch_without_reflog_refreshes_stamp_after_commit(self):
        for package in ("backend", "cli"):
            with self.subTest(package=package):
                _, repo, git, build = self.fixture(package)
                first = self.initialize(git, reflog=False)
                git("pack-refs", "--all", "--prune")
                self.assertFalse((repo / ".git/logs/HEAD").exists())
                self.assertFalse((repo / ".git/refs/heads/main").exists())
                self.assertEqual(build(), (first, True))
                self.assertEqual(build(), (first, True))
                (repo / "README").write_text("second\n")
                git("commit", "-qam", "second")
                second = git("rev-parse", "--short=12", "HEAD")
                self.assertNotEqual(first, second)
                self.assertTrue((repo / ".git/refs/heads/main").is_file())
                self.assertEqual(build(), (second, True))
                self.assertEqual(build(), (second, False))

    def test_no_git_and_explicit_backend_stamp(self):
        for package in ("backend", "cli"):
            with self.subTest(package=package):
                _, _, _, build = self.fixture(package)
                self.assertEqual(build(), ("unknown", True))
                self.assertEqual(build(), ("unknown", False))
                if package == "backend":
                    self.assertEqual(build(extra={"NYXID_GIT_HASH": "explicit"}), ("explicit", True))
                    self.assertEqual(build(extra={"NYXID_GIT_HASH": "explicit"}), ("explicit", False))
                    self.assertEqual(build(extra={"NYXID_GIT_HASH": ""}), ("unknown", True))

    def test_unavailable_reflog_path_does_not_fail_build_or_keep_stale_stamp(self):
        for package in ("backend", "cli"):
            for failure in ("exit 1", "printf '\\377'; exit 0"):
                with self.subTest(package=package, failure=failure):
                    directory, repo, git, build = self.fixture(package)
                    first = self.initialize(git, reflog=False)
                    git("pack-refs", "--all", "--prune")
                    shim_dir = directory / "bin"
                    shim_dir.mkdir()
                    shim = shim_dir / "git"
                    shim.write_text(
                        '#!/bin/sh\nif [ "$1" = rev-parse ] && [ "$2" = --git-path ] '
                        '&& [ "$3" = logs/HEAD ]; then\n'
                        f'{failure}\nfi\nexec {shlex.quote(shutil.which("git"))} "$@"\n'
                    )
                    shim.chmod(0o755)
                    env = {"PATH": str(shim_dir) + os.pathsep + os.environ["PATH"]}
                    self.assertEqual(build(extra=env), (first, True))
                    self.assertEqual(build(extra=env), (first, True))
                    outputs = list((directory / "repo-target/debug/build").glob("*/output"))
                    self.assertEqual(len(outputs), 1)
                    self.assertIn("cargo:rerun-if-changed=build.rs/git-stamp-unavailable",
                                  outputs[0].read_text().splitlines())
                    self.assertTrue((repo / package / "build.rs").is_file())
                    self.assertFalse((repo / package / "build.rs/git-stamp-unavailable").exists())
                    (repo / "README").write_text("second\n")
                    git("commit", "-qam", "second")
                    second = git("rev-parse", "--short=12", "HEAD")
                    self.assertNotEqual(first, second)
                    self.assertEqual(build(extra=env), (second, True))
                    self.assertEqual(build(extra=env), (second, False))

    def test_git_without_path_format_still_watches_commits_and_linked_worktrees(self):
        for package in ("backend", "cli"):
            with self.subTest(package=package):
                directory, repo, git, build = self.fixture(package, old_git=True)
                first = self.initialize(git)
                with self.assertRaises(subprocess.CalledProcessError):
                    git("rev-parse", "--path-format=absolute", "--git-path", "HEAD")
                self.assertEqual(build(), (first, True))
                self.assertEqual(build(), (first, False))
                (repo / "README").write_text("second")
                git("commit", "-qam", "second")
                second = git("rev-parse", "--short=12", "HEAD")
                self.assertEqual(build(), (second, True))
                linked = directory / "linked"
                git("worktree", "add", "-b", "side", str(linked))
                self.assertEqual(build(cwd=linked), (second, True))
                self.assertEqual(build(cwd=linked), (second, False))
                git("checkout", "--detach", first, cwd=linked)
                self.assertEqual(build(cwd=linked), (first, True))
                self.assertEqual(build(cwd=linked), (first, False))

    def test_backend_override_wins_over_git_and_ignores_head_changes(self):
        _, repo, git, build = self.fixture("backend")
        self.initialize(git)
        override = {"NYXID_GIT_HASH": "release-identity"}
        self.assertEqual(build(extra=override), ("release-identity", True))
        (repo / "README").write_text("second")
        git("commit", "-qam", "second")
        self.assertEqual(build(extra=override), ("release-identity", False))
        self.assertEqual(build(), (git("rev-parse", "--short=12", "HEAD"), True))

    @unittest.skipUnless(os.uname().sysname == "Darwin", "macOS SDK environment")
    def test_cli_rechecks_swift_search_path_when_developer_directory_changes(self):
        _, _, git, build = self.fixture("cli")
        stamp = self.initialize(git)
        self.assertEqual(build(), (stamp, True))
        developer = subprocess.check_output(["xcode-select", "-p"], text=True).strip()
        self.assertEqual(build(extra={"DEVELOPER_DIR": developer}), (stamp, True))
        self.assertEqual(build(extra={"DEVELOPER_DIR": developer}), (stamp, False))


if __name__ == "__main__":
    unittest.main()
