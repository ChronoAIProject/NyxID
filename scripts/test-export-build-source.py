#!/usr/bin/env python3
"""Exercise exact committed source exports, tamper rejection and local attributes."""
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("export_build_source", ROOT / "scripts/export-build-source.py")
exporter = importlib.util.module_from_spec(spec)
spec.loader.exec_module(exporter)


class SourceExport(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="nyxid-source-export-test-")
        self.addCleanup(self.temp.cleanup)
        self.directory = Path(self.temp.name)
        self.root = self.directory / "repo"
        self.root.mkdir()
        self.git("init")
        self.git("symbolic-ref", "HEAD", "refs/heads/main")
        (self.root / "source.txt").write_text("exact committed source\n$Format:%H$\n")
        (self.root / "app").mkdir()
        (self.root / "app/run.sh").write_text("#!/bin/sh\nexit 0\n")
        (self.root / "app/run.sh").chmod(0o755)
        (self.root / "alias").symlink_to("source.txt")
        self.commit()

    def git(self, *args):
        with exporter.trusted_git_environment(self.root) as env:
            return subprocess.check_output(
                exporter.git_command(self.root, *args), env=env,
                stderr=subprocess.PIPE).decode().strip()

    def commit(self):
        self.git("add", ".")
        self.git("-c", "user.name=Fixture", "-c", "user.email=fixture@local.invalid",
                 "commit", "-m", "committed source")
        self.commit_id = self.git("rev-parse", "HEAD")

    def exported(self, name="export"):
        destination = self.directory / name
        receipt = exporter.export(self.root, destination, self.commit_id)
        return destination, receipt

    def save_receipt(self, destination, receipt):
        (destination / "source-export.json").write_text(json.dumps(receipt))

    def test_exact_commit_export_is_no_git_and_repeatable(self):
        destination, receipt = self.exported()
        self.assertFalse(exporter.has_git_tree(destination / "source"))
        self.assertEqual((destination / "source/source.txt").read_bytes(),
                         (self.root / "source.txt").read_bytes())
        self.assertTrue((destination / "source/app/run.sh").stat().st_mode & 0o111)
        self.assertEqual(os.readlink(destination / "source/alias"), "source.txt")
        self.assertEqual(exporter.verify(destination, self.commit_id), receipt)
        again, repeated = self.exported("again")
        self.assertEqual((destination / "source.tar").read_bytes(), (again / "source.tar").read_bytes())
        self.assertEqual(receipt, repeated)

    def test_container_ownership_trust_is_exact_and_config_stays_isolated(self):
        hostile = self.directory / "hostile-config"
        hostile.write_text("[safe]\n\tdirectory = *\n")
        with mock.patch.dict(os.environ, {"GIT_CONFIG_GLOBAL": str(hostile),
                                         "XDG_CONFIG_HOME": str(self.directory),
                                         "SUDO_UID": "1001",
                                         "GIT_DIR": str(self.root / ".git"),
                                         "GIT_CONFIG_COUNT": "1",
                                         "GIT_CONFIG_KEY_0": "safe.directory",
                                         "GIT_CONFIG_VALUE_0": "*"}):
            env = exporter.git_environment()
        self.assertEqual(env["GIT_CONFIG_GLOBAL"], os.devnull)
        self.assertEqual(env["GIT_CONFIG_NOSYSTEM"], "1")
        self.assertNotIn("HOME", env)
        self.assertNotIn("XDG_CONFIG_HOME", env)
        self.assertNotIn("SUDO_UID", env)
        self.assertFalse({"GIT_DIR", "GIT_CONFIG_COUNT", "GIT_CONFIG_KEY_0",
                          "GIT_CONFIG_VALUE_0"}.intersection(env))
        # Git's own test switch exercises the ownership check without chown or
        # elevated permissions. The exporter never inherits this variable.
        env["GIT_TEST_ASSUME_DIFFERENT_OWNER"] = "1"
        denied = subprocess.run(["git", "-C", str(self.root), "status", "--porcelain"],
                                env=env, capture_output=True, text=True)
        actual_ownership = denied.returncode == 0
        if actual_ownership:
            # Git 2.25 lacks the test switch. The offline container control
            # runs as root and exercises the real ownership check instead.
            self.assertEqual(os.geteuid(), 0, "ownership control needs Git's test switch or container root")
            for path in [self.root, *self.root.rglob("*")]:
                os.lchown(path, 1001, 1001)
            denied = subprocess.run(["git", "-C", str(self.root), "status", "--porcelain"],
                                    env=env, capture_output=True, text=True)
        self.assertNotEqual(denied.returncode, 0)
        self.assertIn("dubious ownership", denied.stderr)
        with mock.patch.object(exporter, "git_environment", return_value=env):
            destination, receipt = self.exported()
            self.assertEqual(exporter.verify(destination, self.commit_id), receipt)
            other = self.directory / "unrelated"
            subprocess.run(["git", "init", str(other)], env=env, check=True, capture_output=True)
            if actual_ownership:
                for path in [other, *other.rglob("*")]:
                    os.lchown(path, 1001, 1001)
            with exporter.trusted_git_environment(self.root) as trusted:
                config = Path(trusted["GIT_CONFIG_GLOBAL"])
                self.assertEqual(config, Path(trusted["XDG_CONFIG_HOME"]) / "git/config")
                self.assertEqual(subprocess.check_output(
                    ["git", "config", "--file", str(config), "--get-all", "safe.directory"],
                    env=env, text=True).splitlines(), [str(self.root.resolve())])
                denied = subprocess.run(exporter.git_command(self.root, "-C", str(other),
                                                            "status", "--porcelain"),
                                        env=trusted, capture_output=True, text=True)
            self.assertNotEqual(denied.returncode, 0)
            self.assertIn("dubious ownership", denied.stderr)
            # Trusting a candidate subdirectory must not silently classify its
            # differently owned parent repository as a successful no-Git check.
            nested = other / "nested"
            nested.mkdir()
            with self.assertRaisesRegex(ValueError, "Git boundary"):
                exporter.has_git_tree(nested)

    def test_literal_star_root_cannot_expand_trust_to_sibling_repositories(self):
        star = self.directory / "*"
        self.root.rename(star)
        other = self.directory / "unrelated"
        subprocess.run(["git", "init", str(other)], env=exporter.git_environment(),
                       check=True, capture_output=True)
        # Refuse both trust representations before any Git command/config can
        # use /parent/* to authorize this unrelated sibling. Resolving a symlink
        # alias must retain the same refusal.
        alias = self.directory / "star-alias"
        alias.symlink_to(star, target_is_directory=True)
        for root in (star, alias):
            with self.subTest(root=root):
                with self.assertRaisesRegex(ValueError, "wildcard"):
                    exporter.git_command(root, "-C", str(other), "status", "--porcelain")
                with self.assertRaisesRegex(ValueError, "wildcard"):
                    with exporter.trusted_git_environment(root):
                        self.fail("literal-star root received Git trust")
                with self.assertRaisesRegex(ValueError, "wildcard"):
                    exporter.export(root, self.directory / "export", self.commit_id)

    def test_trust_config_paths_preserve_exact_value_without_injection(self):
        original = self.root
        for name in ('has space', 'has"quote', 'has\\backslash', 'has\nnewline',
                     'has\ttab', 'injection"\n[safe]\n directory = *'):
            with self.subTest(name=name):
                selected = self.directory / name
                self.root.rename(selected)
                self.root = selected
                try:
                    with exporter.trusted_git_environment(selected) as env:
                        values = subprocess.check_output(
                            ["git", "config", "--file", env["GIT_CONFIG_GLOBAL"],
                             "--null", "--get-all", "safe.directory"], env=env)
                        self.assertEqual(values, str(selected.resolve()).encode() + b"\0")
                        self.assertEqual(subprocess.check_output(
                            exporter.git_command(selected, "rev-parse", "HEAD"),
                            env=env).decode().strip(), self.commit_id)
                finally:
                    self.root.rename(original)
                    self.root = original
        for name in ('has\rcarriage-return', 'has\x01control', 'has\x7fdelete'):
            selected = self.directory / name
            selected.mkdir()
            with self.subTest(name=name):
                with self.assertRaisesRegex(ValueError, "control character"):
                    exporter.git_command(selected, "status")
                with self.assertRaisesRegex(ValueError, "control character"):
                    with exporter.trusted_git_environment(selected):
                        self.fail("unsupported control received Git trust")

    def test_inherited_home_xdg_includes_are_never_read(self):
        home = self.directory / "hostile-home"
        xdg = self.directory / "hostile-xdg"
        home.mkdir()
        (xdg / "git").mkdir(parents=True)
        # Reading either include would fail with invalid config syntax.
        invalid = self.directory / "invalid-include"
        invalid.write_text("[invalid\n")
        include = '[include]\n path = "' + str(invalid) + '"\n'
        (home / ".gitconfig").write_text(include)
        (xdg / "git/config").write_text(include)
        with mock.patch.dict(os.environ, {"HOME": str(home), "XDG_CONFIG_HOME": str(xdg),
                                         "GIT_CONFIG_GLOBAL": str(home / ".gitconfig"),
                                         "SUDO_UID": "1001", "GIT_CONFIG_COUNT": "1",
                                         "GIT_CONFIG_KEY_0": "include.path",
                                         "GIT_CONFIG_VALUE_0": str(invalid)}):
            self.assertEqual(exporter.git(self.root, "rev-parse", "HEAD").decode().strip(),
                             self.commit_id)

    def test_private_config_is_removed_on_body_and_git_errors(self):
        for git_error in (False, True):
            with self.subTest(git_error=git_error):
                with self.assertRaises((RuntimeError, subprocess.CalledProcessError)):
                    with exporter.trusted_git_environment(self.root) as env:
                        config = Path(env["GIT_CONFIG_GLOBAL"])
                        directory = Path(env["XDG_CONFIG_HOME"])
                        self.assertTrue(config.is_file())
                        if git_error:
                            subprocess.check_output(exporter.git_command(self.root, "not-a-command"),
                                                    env=env, stderr=subprocess.PIPE)
                        raise RuntimeError("fixture body error")
                self.assertFalse(config.exists())
                self.assertFalse(directory.exists())

    def test_dirty_untracked_and_wrong_commit_fail(self):
        with self.assertRaisesRegex(ValueError, "full commit"):
            exporter.export(self.root, self.directory / "wrong", "a" * 40)
        (self.root / "source.txt").write_text("unstaged input")
        with self.assertRaisesRegex(ValueError, "clean checkout"):
            self.exported("dirty")
        self.git("restore", "source.txt")
        (self.root / "new.txt").write_text("untracked input")
        with self.assertRaisesRegex(ValueError, "clean checkout"):
            self.exported("untracked")

    def test_output_in_any_git_tree_or_nonroot_checkout_fails(self):
        with self.assertRaisesRegex(ValueError, "outside the checkout"):
            exporter.export(self.root, self.root / "export", self.commit_id)
        with self.assertRaisesRegex(ValueError, "checkout root"):
            exporter.export(self.root / "app", self.directory / "subdir", self.commit_id)
        other = self.directory / "other"
        other.mkdir()
        subprocess.run(["git", "init", str(other)], env=exporter.git_environment(),
                       check=True, capture_output=True)
        with self.assertRaisesRegex(ValueError, "outside any Git"):
            exporter.export(self.root, other / "export", self.commit_id)

    def test_local_and_committed_archive_attributes_cannot_omit_or_transform_inputs(self):
        (self.root / ".gitattributes").write_text("source.txt export-subst\n")
        self.commit()
        first, receipt = self.exported()
        (self.root / ".git/info/attributes").write_text("source.txt export-ignore\n")
        self.assertEqual(self.git("status", "--porcelain"), "")
        altered = subprocess.check_output(exporter.git_command(
            self.root, "archive", "--format=tar", self.commit_id), env=exporter.git_environment())
        with tarfile.open(fileobj=io.BytesIO(altered)) as tar:
            self.assertNotIn("source.txt", tar.getnames())  # Reproduce ordinary git-archive omission.
        second, protected = self.exported("with-local-attributes")
        self.assertEqual((first / "source.tar").read_bytes(), (second / "source.tar").read_bytes())
        self.assertEqual(receipt, protected)
        self.assertIn("$Format:%H$", (second / "source/source.txt").read_text())

    def test_new_source_file_or_empty_directory_and_missing_source_fail(self):
        destination, _ = self.exported()
        new = destination / "source/new-input"
        new.write_text("new source input")
        with self.assertRaisesRegex(ValueError, "missing/extra"):
            exporter.verify(destination, self.commit_id)
        new.unlink()
        new.mkdir()
        with self.assertRaisesRegex(ValueError, "missing/extra"):
            exporter.verify(destination, self.commit_id)
        new.rmdir()
        (destination / "source/source.txt").unlink()
        with self.assertRaisesRegex(ValueError, "missing/extra"):
            exporter.verify(destination, self.commit_id)

    def test_reviewed_generated_paths_cannot_hide_tracked_source(self):
        destination, receipt = self.exported()
        (destination / "source/target").mkdir()
        (destination / "source/target/output").write_text("build output")
        with self.assertRaises(ValueError):
            exporter.verify(destination, self.commit_id)
        self.assertEqual(exporter.verify(destination, self.commit_id, ["target"]), receipt)
        with self.assertRaises(ValueError):
            exporter.verify(destination, self.commit_id, ["target", "app"])
        with self.assertRaises(ValueError):
            exporter.verify(destination, self.commit_id, ["../source"])

    def test_file_mode_symlink_and_content_tamper_fail_even_with_edited_file_receipt(self):
        destination, receipt = self.exported()
        source = destination / "source/source.txt"
        source.write_text("altered source")
        receipt["files"] = exporter.inventory(destination / "source")
        self.save_receipt(destination, receipt)
        with self.assertRaisesRegex(ValueError, "committed blob"):
            exporter.verify(destination, self.commit_id)
        source.write_bytes((self.root / "source.txt").read_bytes())
        (destination / "source/app/run.sh").chmod(0o644)
        with self.assertRaisesRegex(ValueError, "committed mode"):
            exporter.verify(destination, self.commit_id)
        (destination / "source/app/run.sh").chmod(0o755)
        alias = destination / "source/alias"
        alias.unlink()
        alias.symlink_to("../outside")
        with self.assertRaisesRegex(ValueError, "escapes"):
            exporter.verify(destination, self.commit_id)

    def test_archive_and_tree_receipt_tamper_fail(self):
        destination, receipt = self.exported()
        archive = destination / "source.tar"
        archive.write_bytes(archive.read_bytes() + b"tampered")
        with self.assertRaisesRegex(ValueError, "archive differs"):
            exporter.verify(destination, self.commit_id)
        # Updating the recorded archive hash still cannot replace committed bytes.
        with tarfile.open(archive, "w") as tar:
            info = tarfile.TarInfo("source.txt")
            info.size = 4
            tar.addfile(info, io.BytesIO(b"evil"))
        receipt["archive"].update(sha256=exporter.digest(archive), size=archive.stat().st_size)
        self.save_receipt(destination, receipt)
        with self.assertRaisesRegex(ValueError, "committed blob"):
            exporter.verify(destination, self.commit_id)
        receipt["committed_files"][0]["git_oid"] = "a" * 40
        self.save_receipt(destination, receipt)
        with self.assertRaisesRegex(ValueError, "committed tree"):
            exporter.verify(destination, self.commit_id)

    def test_safe_directory_symlink_and_escaping_committed_symlinks(self):
        (self.root / "tools").symlink_to("app", target_is_directory=True)
        self.commit()
        destination, receipt = self.exported()
        self.assertEqual(exporter.verify(destination, self.commit_id), receipt)
        (self.root / "escape").symlink_to("../outside")
        self.commit()
        with self.assertRaisesRegex(ValueError, "escapes"):
            self.exported("escape")

    def test_commit_object_duplicate_json_and_wrong_expected_identity_fail(self):
        destination, receipt = self.exported()
        with self.assertRaisesRegex(ValueError, "expected identity"):
            exporter.verify(destination, "a" * 40)
        receipt["source"]["epoch"] += 1
        self.save_receipt(destination, receipt)
        with self.assertRaisesRegex(ValueError, "tree/epoch"):
            exporter.verify(destination, self.commit_id)
        (destination / "source-export.json").write_text('{"schema":1,"schema":2}')
        with self.assertRaisesRegex(ValueError, "duplicate"):
            exporter.verify(destination, self.commit_id)


if __name__ == "__main__":
    unittest.main()
