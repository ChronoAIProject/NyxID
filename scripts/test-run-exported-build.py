#!/usr/bin/env python3
"""Exercise source verification around real successful, failed and cancelled builds."""
import importlib.util
import json
from pathlib import Path
import sys
import unittest

ROOT = Path(__file__).resolve().parents[1]


def module(name, filename):
    spec = importlib.util.spec_from_file_location(name, ROOT / "scripts" / filename)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


runner = module("run_exported_build", "run-exported-build.py")
fixtures = module("source_export_fixtures", "test-export-build-source.py")
contract = module("publication_contract", "publication_contract.py")


class ExportedBuild(unittest.TestCase):
    def setUp(self):
        self.fixture = fixtures.SourceExport("test_exact_commit_export_is_no_git_and_repeatable")
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)
        self.destination, self.exported = self.fixture.exported()
        self.receipt = self.fixture.directory / "execution.json"
        self.identity = {key: self.exported["source"][key] for key in ("commit", "epoch")}

    def run_build(self, program, generated=(), cwd=None):
        return runner.run(self.destination, self.fixture.commit_id, self.receipt,
                          [sys.executable, "-c", program], generated, cwd)

    def accept(self):
        receipt = json.loads(self.receipt.read_text())
        return contract.require_build_execution(receipt, self.identity, self.exported["archive"]["sha256"])

    def test_success_uses_exact_receipt_identity_and_before_after_source(self):
        self.assertEqual(self.run_build(
            "import os; from pathlib import Path; "
            "Path('target').mkdir(); Path('target/identity').write_text("
            "os.environ['NYXID_GIT_HASH']+' '+os.environ['SOURCE_DATE_EPOCH'])",
            ["target"]), 0)
        self.accept()
        self.assertEqual((self.destination / "source/target/identity").read_text(),
                         self.identity["commit"] + " " + str(self.identity["epoch"]))

    def test_changed_source_is_rejected_before_any_command_runs(self):
        (self.destination / "source/source.txt").write_text("tampered")
        sentinel = self.fixture.directory / "ran"
        with self.assertRaises(ValueError):
            self.run_build("from pathlib import Path; Path(" + repr(str(sentinel)) + ").touch()")
        self.assertFalse(sentinel.exists())

    def test_command_cannot_change_source_then_claim_success(self):
        with self.assertRaisesRegex(ValueError, "changed authenticated"):
            self.run_build("from pathlib import Path; Path('source.txt').write_text('modified')")
        receipt = json.loads(self.receipt.read_text())
        self.assertEqual(receipt["exit_code"], 0)
        self.assertFalse(receipt["post_build_source_verified"])
        with self.assertRaises(ValueError):
            self.accept()

    def test_failed_build_with_preexisting_artifact_cannot_satisfy_gate(self):
        (self.destination / "source/target").mkdir()
        artifact = self.destination / "source/target/archive.tar.gz"
        artifact.write_bytes(b"pre-existing archive")
        self.assertEqual(self.run_build("raise SystemExit(9)", ["target"]), 9)
        self.assertEqual(artifact.read_bytes(), b"pre-existing archive")
        with self.assertRaises(ValueError):
            self.accept()

    def test_cancelled_build_with_preexisting_artifact_cannot_satisfy_gate(self):
        (self.destination / "source/target").mkdir()
        artifact = self.destination / "source/target/archive.tar.gz"
        artifact.write_bytes(b"pre-existing archive")
        self.assertEqual(self.run_build(
            "import os,signal; os.kill(os.getpid(),signal.SIGTERM)", ["target"]), 143)
        self.assertTrue(artifact.is_file())
        with self.assertRaises(ValueError):
            self.accept()

    def test_preexisting_receipt_and_tracked_source_exclusions_fail(self):
        self.receipt.write_text("stale success")
        with self.assertRaises(FileExistsError):
            self.run_build("raise SystemExit(0)")
        self.receipt.unlink()
        with self.assertRaises(ValueError):
            self.run_build("raise SystemExit(0)", ["app"])

    def test_source_relative_cwd_and_external_receipt_boundary(self):
        self.assertEqual(self.run_build("from pathlib import Path; assert Path('run.sh').is_file()",
                                        cwd="app"), 0)
        self.accept()
        with self.assertRaises(ValueError):
            runner.run(self.destination, self.fixture.commit_id,
                       self.destination / "source/receipt.json", [sys.executable, "-c", "pass"])


if __name__ == "__main__":
    unittest.main()
