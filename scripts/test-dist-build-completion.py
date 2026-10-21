#!/usr/bin/env python3
"""Exercise required child completion despite an outer success and cached artifacts."""
import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


def module(name, filename):
    spec = importlib.util.spec_from_file_location(name, ROOT / "scripts" / filename)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


guard = module("required_cargo_guard", "guard-dist-cargo.py")
producer = module("required_dist_build", "run-dist-build.py")
fixtures = module("completion_export_fixtures", "test-export-build-source.py")
contracts = module("completion_plan_fixtures", "test_publication_contract.py")


class DistCompletion(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="nyxid-dist-required-build-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.binary = self.root / "nyxid"
        self.binary.write_bytes(b"same-source pre-existing compiled artifact")
        self.receipts = self.root / "cargo"
        self.receipts.mkdir()
        self.target = "aarch64-apple-darwin"
        self.identity = {"commit": "a" * 40, "epoch": 1792367116}
        self.env = {**os.environ, "NYXID_GIT_HASH": self.identity["commit"],
                    "SOURCE_DATE_EPOCH": str(self.identity["epoch"]),
                    "NYXID_DIST_TARGETS": json.dumps([self.target]),
                    "NYXID_DIST_CARGO_RECEIPTS": str(self.receipts)}
        self.arguments = ["auditable", "build", "--target", self.target, "--profile", "dist",
                          "--message-format=json-render-diagnostics", "--package", "nyxid-cli"]

    def fake_cargo(self, completion, code):
        script = self.root / "fixture-cargo"
        script.write_text("#!" + sys.executable + "\n" +
                          "import json,sys,os\nassert '--locked' in sys.argv\n" +
                          "assert os.environ['CARGO']==sys.argv[0]\n" +
                          "print(json.dumps(" + repr({"reason": "compiler-artifact", "package_id": "path+file:///fixture/cli#nyxid-cli@0.71.0",
                            "target": {"name": "nyxid", "kind": ["bin"]}, "executable": str(self.binary)}) + "))\n" +
                          ("print(json.dumps({'reason':'build-finished','success':" + repr(completion) + "}))\n" if completion is not None else "") +
                          "raise SystemExit(" + str(code) + ")\n")
        script.chmod(0o755)
        self.env["NYXID_DIST_REAL_CARGO"] = str(script)

    def invoke_outer(self):
        # Same observed failure shape as dist: compiler failure/cancellation is
        # ignored, outer process succeeds and a same-source old binary remains.
        env = {**self.env, "TEST_GUARD": str(ROOT / "scripts/guard-dist-cargo.py"),
               "TEST_ARGS": json.dumps(self.arguments)}
        result = subprocess.run([sys.executable, "-c",
            "import json,os,subprocess,sys; "
            "subprocess.run([sys.executable,os.environ['TEST_GUARD'],*json.loads(os.environ['TEST_ARGS'])]); "
            "raise SystemExit(0)"], env=env, capture_output=True)
        self.assertEqual(result.returncode, 0)
        self.assertEqual(self.binary.read_bytes(), b"same-source pre-existing compiled artifact")

    def test_failed_or_cancelled_required_compiler_outer_zero_and_old_artifact_rejected(self):
        for completion, code in ((False, 9), (None, 143), (True, 9), (None, 0)):
            with self.subTest(completion=completion, code=code):
                for path in self.receipts.iterdir():
                    path.unlink()
                self.fake_cargo(completion, code)
                self.invoke_outer()
                with self.assertRaisesRegex(ValueError, "required nested Cargo build"):
                    producer.required_children(self.receipts, [self.target], self.identity)

    def test_locked_success_requires_exact_target_and_completion_stream(self):
        self.fake_cargo(True, 0)
        self.invoke_outer()
        result = producer.required_children(self.receipts, [self.target], self.identity)
        self.assertIn("--locked", result[self.target]["command"])
        with self.assertRaisesRegex(ValueError, "coverage"):
            producer.required_children(self.receipts, [self.target, self.target], self.identity)
        message = next(self.receipts.glob("*.jsonl"))
        message.write_text(message.read_text().replace('"success": true', '"success": false'))
        with self.assertRaisesRegex(ValueError, "evidence changed"):
            producer.required_children(self.receipts, [self.target], self.identity)

    def test_missing_child_completion_and_fresh_staging_reject_previous_artifact(self):
        with self.assertRaisesRegex(ValueError, "coverage"):
            producer.required_children(self.receipts, [self.target], self.identity)
        fixture = fixtures.SourceExport("test_exact_commit_export_is_no_git_and_repeatable")
        fixture.setUp()
        self.addCleanup(fixture.doCleanups)
        destination, exported = fixture.exported()
        stage = destination / "source/target/distrib"
        stage.mkdir(parents=True)
        (stage / "old.tar.gz").write_bytes(b"pre-existing archive from same source")
        plan = contracts.cli_plan()
        plan["source"] = {key: exported["source"][key] for key in ("commit", "epoch")}
        with self.assertRaisesRegex(ValueError, "fresh artifact staging"):
            producer.run(destination, plan, self.root / "evidence", "local", [self.target], dist="must-not-run")
        self.assertTrue((stage / "old.tar.gz").is_file())

    def test_unexpected_build_policy_cannot_bypass_guard(self):
        self.fake_cargo(True, 0)
        for arguments in (["build"], ["auditable", "build", "--workspace"],
                          [*self.arguments, "--target", "x86_64-unknown-linux-musl"],
                          [*self.arguments, "--workspace"]):
            with self.subTest(arguments=arguments), self.assertRaises(ValueError):
                guard.invoke(arguments, self.env)

    def test_restored_empty_stage_directories_are_removed_without_touching_dependency_cache(self):
        tree = self.root / "source"
        (tree / "target/distrib/nyxid-aarch64-apple-darwin").mkdir(parents=True)
        (tree / "target/release-source/empty").mkdir(parents=True)
        dependency = tree / "target/aarch64-apple-darwin/dist/deps/cached.rlib"
        dependency.parent.mkdir(parents=True)
        dependency.write_bytes(b"compiled dependency cache")
        removed = producer.fresh_staging(tree)
        self.assertEqual(removed, ["target/distrib", "target/distrib/nyxid-aarch64-apple-darwin",
                                   "target/release-source", "target/release-source/empty"])
        self.assertFalse((tree / "target/distrib").exists())
        self.assertFalse((tree / "target/release-source").exists())
        self.assertEqual(dependency.read_bytes(), b"compiled dependency cache")

    def test_restored_stale_or_symlink_stage_is_rejected_and_preserved(self):
        tree = self.root / "source"
        empty = tree / "target/distrib/empty"
        empty.mkdir(parents=True)
        source_stage = tree / "target/release-source"
        source_stage.mkdir()
        stale = source_stage / "source.tar.gz"
        stale.write_bytes(b"same-source pre-existing source archive")
        with self.assertRaisesRegex(ValueError, "fresh artifact staging"):
            producer.fresh_staging(tree)
        self.assertTrue(empty.is_dir())
        self.assertEqual(stale.read_bytes(), b"same-source pre-existing source archive")
        stale.unlink()
        stale.symlink_to(self.binary)
        with self.assertRaisesRegex(ValueError, "fresh artifact staging"):
            producer.fresh_staging(tree)
        self.assertTrue(stale.is_symlink())
        self.assertTrue(empty.is_dir())


if __name__ == "__main__":
    unittest.main()
