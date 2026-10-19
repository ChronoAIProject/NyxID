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
        self.git("init", "-b", "main")
        (self.root / "source.txt").write_text("exact committed source\n$Format:%H$\n")
        (self.root / "app").mkdir()
        (self.root / "app/run.sh").write_text("#!/bin/sh\nexit 0\n")
        (self.root / "app/run.sh").chmod(0o755)
        (self.root / "alias").symlink_to("source.txt")
        self.commit()

    def git(self, *args):
        return subprocess.check_output(
            exporter.git_command(self.root, *args), env=exporter.git_environment(),
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
