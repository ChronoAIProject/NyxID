"""Pin the portable usage contract bundle in the fast PR CI gate too."""

import hashlib
import json
from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[2]
BUNDLE = ROOT / "backend/tests/fixtures/cma_usage_v1"
MANIFEST_SHA256 = "2f189190dd24595d0b261a56030be38d2328df8c285d35fec08c3e3293948797"


class UsageContractBundleTests(unittest.TestCase):
    def test_release_manifest_and_all_files_are_pinned(self):
        raw = (BUNDLE / "manifest.json").read_bytes()
        self.assertEqual(hashlib.sha256(raw).hexdigest(), MANIFEST_SHA256)
        manifest = json.loads(raw)
        self.assertEqual(manifest["contract"], "cma-usage-v1")
        self.assertEqual(manifest["canonicalization"], "RFC8785")
        self.assertEqual(
            {path.name for path in BUNDLE.iterdir()} - {"manifest.json"},
            set(manifest["files"]),
        )
        for name, digest in manifest["files"].items():
            with self.subTest(file=name):
                self.assertEqual(
                    hashlib.sha256((BUNDLE / name).read_bytes()).hexdigest(), digest
                )
        # These manifest keys and digest values are ASCII; no JSON numbers occur.
        files = json.dumps(manifest["files"], sort_keys=True, separators=(",", ":"))
        self.assertEqual(
            hashlib.sha256(files.encode()).hexdigest(), manifest["bundle_sha256"]
        )
        for entry in manifest["fixtures"]:
            with self.subTest(fixture=entry["name"]):
                canonical = (BUNDLE / entry["canonical_file"]).read_bytes()
                self.assertEqual(
                    hashlib.sha256(canonical).hexdigest(), entry["canonical_sha256"]
                )
                self.assertEqual(
                    json.loads(canonical), json.loads((BUNDLE / entry["file"]).read_bytes())
                )


if __name__ == "__main__":
    unittest.main()
