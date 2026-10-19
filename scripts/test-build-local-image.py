#!/usr/bin/env python3
"""Exercise exact source/argument flow without building an image or invoking a daemon."""
import importlib.util
import json
import os
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]


def module(name, filename):
    spec = importlib.util.spec_from_file_location(name, ROOT / "scripts" / filename)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


fixtures = module("local_image_fixtures", "test-export-build-source.py")
builder = module("local_image_builder", "build-local-image.py")


class LocalImage(unittest.TestCase):
    def setUp(self):
        self.fixture = fixtures.SourceExport("test_exact_commit_export_is_no_git_and_repeatable")
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)
        (self.fixture.root / "Dockerfile").write_text("FROM scratch\nCOPY source.txt /source.txt\n")
        self.fixture.commit()
        self.destination = self.fixture.directory / "image-export"
        self.record = self.fixture.directory / "docker-arguments.json"
        executable = self.fixture.directory / "bin/docker"
        executable.parent.mkdir()
        executable.write_text("#!" + sys.executable + "\nimport json,sys,os\nfrom pathlib import Path\n"
                              "Path(os.environ['TEST_IMAGE_ARGUMENTS']).write_text(json.dumps({'argv':sys.argv[1:],'commit':os.environ['NYXID_GIT_HASH'],'epoch':os.environ['SOURCE_DATE_EPOCH']}))\n")
        executable.chmod(0o755)
        self.environment = patch.dict(os.environ, {"PATH": str(executable.parent) + os.pathsep + os.environ["PATH"],
                                                   "TEST_IMAGE_ARGUMENTS": str(self.record)})
        self.environment.start()
        self.addCleanup(self.environment.stop)

    def test_exact_export_commit_epoch_file_platform_target_and_named_context(self):
        code, exported = builder.build(self.fixture.root, self.destination, "Dockerfile", "fixture:local",
                                       "runtime", "linux/arm64", ["cma=/separately-reviewed/context"])
        self.assertEqual(code, 0)
        args = json.loads(self.record.read_text())
        commit = self.fixture.commit_id
        epoch = str(exported["source"]["epoch"])
        self.assertEqual(args, {"argv": ["build", "--build-arg", "NYXID_GIT_HASH=" + commit,
             "--build-arg", "SOURCE_DATE_EPOCH=" + epoch, "--file", str(self.destination / "source/Dockerfile"),
             "--tag", "fixture:local", "--target", "runtime", "--platform", "linux/arm64",
             "--build-context", "cma=/separately-reviewed/context", str(self.destination / "source")],
             "commit": commit, "epoch": epoch})
        receipt = json.loads((self.destination / "image-build-execution.json").read_text())
        self.assertTrue(receipt["post_build_source_verified"])
        self.assertEqual(receipt["exit_code"], 0)

    def test_dirty_checkout_cannot_invoke_docker(self):
        (self.fixture.root / "source.txt").write_text("uncommitted source")
        with self.assertRaisesRegex(ValueError, "clean checkout"):
            builder.build(self.fixture.root, self.destination, "Dockerfile", "fixture:local")
        self.assertFalse(self.record.exists())

    def test_uncommitted_or_escaping_dockerfile_cannot_be_used(self):
        with self.assertRaises(ValueError):
            builder.build(self.fixture.root, self.destination, "../Dockerfile", "fixture:local")
        self.assertFalse(self.record.exists())


if __name__ == "__main__":
    unittest.main()
