#!/usr/bin/env python3
"""Adversarial regressions for the build-input admission boundary."""
import hashlib
import importlib.util
import io
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]


def module(name):
    spec = importlib.util.spec_from_file_location(name.replace("-", "_"), ROOT / "scripts" / (name + ".py"))
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


policy = module("check-build-inputs")
bootstrap = module("bootstrap-build-tool")


class BuildPolicy(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        for directory in (".github/workflows", "security"):
            shutil.copytree(ROOT / directory, self.root / directory)
        self.dockerfiles = subprocess.check_output(["git", "ls-files", "*Dockerfile*"], cwd=ROOT, text=True).splitlines()
        for name in self.dockerfiles:
            target = self.root / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / name, target)
        self.mock = patch.object(policy.subprocess, "check_output", return_value="\n".join(self.dockerfiles))
        self.mock.start()
        self.addCleanup(self.mock.stop)
        self.addCleanup(self.temporary.cleanup)

    def mutate(self, name, old, new, reason):
        path = self.root / name
        text = path.read_text()
        self.assertIn(old, text)
        path.write_text(text.replace(old, new, 1))
        self.assertTrue(any(reason in error for error in policy.check(self.root)))

    def test_reviewed_inputs_pass(self):
        self.assertEqual(policy.check(self.root), [])

    def test_mutable_action_and_unknown_commit_rejected(self):
        name = ".github/workflows/release.yml"
        old = "actions/checkout@d23441a48e516b6c34aea4fa41551a30e30af803"
        self.mutate(name, old, "actions/checkout@v6", "unpinned Action")
        self.mutate(name, "actions/checkout@v6", "actions/checkout@" + "a" * 40, "lacks resolution receipt")

    def test_mutable_base_rejected(self):
        self.mutate("frontend/Dockerfile", "node:22.23.2-alpine@sha256:b6f26b36c8ff49624cfdac716b8ea1138d606df02586a77d364bb5536a634f85", "node:22.23.2-alpine", "unpinned base")

    def test_generated_installer_rejected(self):
        self.mutate(".github/workflows/release.yml", "python3 scripts/bootstrap-build-tool.py cargo-dist", "${{ matrix.install_dist.run }} #", "executable generated")

    def test_unlocked_rust_build_rejected(self):
        self.mutate("backend/Dockerfile", "cargo build --locked", "cargo build", "does not enforce lockfile")

    def test_download_checksum_failure(self):
        response = io.BytesIO(b"well-formed executable with altered bytes")
        response.url = "https://github.com/official/release"
        with patch.object(bootstrap.urllib.request, "urlopen", return_value=response):
            with self.assertRaisesRegex(ValueError, "checksum mismatch"):
                bootstrap.download_verified({"url": response.url, "sha256": hashlib.sha256(b"reviewed bytes").hexdigest()}, self.root / "download")

    def test_unsafe_npm_archive_rejected(self):
        path = self.root / "npm.tar.gz"
        with tarfile.open(path, "w:gz") as archive:
            item = tarfile.TarInfo("package/../../outside")
            item.size = 1
            archive.addfile(item, io.BytesIO(b"x"))
        with self.assertRaisesRegex(ValueError, "invalid npm archive path"):
            bootstrap.unpack_npm(path, self.root / "npm")


if __name__ == "__main__":
    unittest.main()
