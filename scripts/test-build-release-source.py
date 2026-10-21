#!/usr/bin/env python3
"""Check release source bytes against raw committed exports and rejection controls."""
import gzip
import importlib.util
from pathlib import Path
import tarfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


def module(name, filename):
    spec = importlib.util.spec_from_file_location(name, ROOT / "scripts" / filename)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


fixtures = module("release_source_fixtures", "test-export-build-source.py")
builder = module("release_source_builder", "build-release-source.py")


class ReleaseSource(unittest.TestCase):
    def setUp(self):
        self.fixture = fixtures.SourceExport("test_exact_commit_export_is_no_git_and_repeatable")
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)
        self.destination, self.exported = self.fixture.exported()
        self.tree = self.destination / "source"
        self.commit = self.exported["source"]["commit"]
        self.epoch = self.exported["source"]["epoch"]

    def build(self):
        return builder.build(self.tree, self.commit, self.epoch)

    def test_exact_raw_tar_gzip_repeatability_modes_symlinks_and_checksum(self):
        receipt = self.build()
        archive = self.tree / builder.OUTPUT / "source.tar.gz"
        self.assertEqual(gzip.decompress(archive.read_bytes()), (self.destination / "source.tar").read_bytes())
        with tarfile.open(archive) as tar:
            self.assertTrue(tar.getmember("alias").issym())
            self.assertEqual(tar.getmember("app/run.sh").mode, 0o755)
        other, _ = self.fixture.exported("repeat")
        repeated = builder.build(other / "source", self.commit, self.epoch)
        self.assertEqual(receipt, repeated)
        self.assertEqual(archive.read_bytes(), (other / "source" / builder.OUTPUT / "source.tar.gz").read_bytes())
        self.assertEqual((archive.parent / "source.tar.gz.sha256").read_text(), receipt["subject"]["sha256"] + " *source.tar.gz\n")

    def test_wrong_source_epoch_generated_source_and_missing_receipt_fail(self):
        with self.assertRaises(ValueError):
            builder.build(self.tree, "b" * 40, self.epoch)
        with self.assertRaises(ValueError):
            builder.build(self.tree, self.commit, self.epoch + 1)
        (self.tree / "new.txt").touch()
        with self.assertRaises(ValueError):
            self.build()
        (self.tree / "new.txt").unlink()
        (self.destination / "source-export.json").unlink()
        with self.assertRaises(FileNotFoundError):
            self.build()

    def test_stale_outputs_never_reused_after_hook_failure(self):
        self.build()
        with self.assertRaises(FileExistsError):
            self.build()
        # Old output remains forensic evidence; a final producer additionally
        # requires this invocation's successful receipt and fresh staging.
        self.assertTrue((self.tree / builder.OUTPUT / "source.tar.gz").is_file())

    def test_changed_tar_members_and_compressed_trailing_payload_fail(self):
        self.build()
        archive = self.tree / builder.OUTPUT / "source.tar.gz"
        archive.write_bytes(archive.read_bytes() + gzip.compress(b"new content"))
        with self.assertRaises(ValueError):
            builder.inspect(archive, self.exported)
        archive.write_bytes(gzip.compress(b"different tar"))
        with self.assertRaises(ValueError):
            builder.inspect(archive, self.exported)


if __name__ == "__main__":
    unittest.main()
