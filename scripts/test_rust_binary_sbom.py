import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("binary_sbom", Path(__file__).with_name("rust-binary-sbom.py"))
binary_sbom = importlib.util.module_from_spec(spec)
spec.loader.exec_module(binary_sbom)


class RustBinarySbomTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.binary = Path(temporary.name) / "nyxid"
        self.binary.write_bytes(b"final binary fixture")
        self.receipt = {"schema": "nyxid.rust-binary-info.v1", "subject": {"name": "nyxid", "size": self.binary.stat().st_size,
            "sha256": hashlib.sha256(self.binary.read_bytes()).hexdigest()}, "audit": {"format": 1, "packages": [
                {"name": "nyxid-cli", "version": "0.71.0", "source": "local", "root": True, "dependencies": [1]},
                {"name": "fixture-macro", "version": "1.0.0", "source": "crates.io", "kind": "build"}]}}

    def generate(self):
        return binary_sbom.sbom(self.binary, self.receipt, "nyxid-cli", "a" * 40)

    def test_subject_and_dependency_kind_remain_explicit(self):
        bom = self.generate()
        self.assertEqual(bom["metadata"]["component"]["hashes"][0]["content"], self.receipt["subject"]["sha256"])
        self.assertIn({"name": "nyxid:compiled-dependency-kind", "value": "build"}, bom["components"][1]["properties"])
        self.assertNotIn("purl", bom["components"][0])
        self.assertEqual(bom["components"][1]["purl"], "pkg:cargo/fixture-macro@1.0.0")
        self.assertTrue(bom["components"][0]["externalReferences"][0]["url"].endswith("a" * 40))
        self.assertNotIn(str(self.binary.parent), json.dumps(bom))

    def test_stale_or_unrelated_subject_is_rejected(self):
        self.binary.write_bytes(b"changed final bytes")
        with self.assertRaisesRegex(ValueError, "actual subject bytes"):
            self.generate()

    def test_unsupported_git_or_custom_registry_origin_fails(self):
        for origin in ("git", "registry"):
            self.receipt["audit"]["packages"][1]["source"] = origin
            with self.assertRaisesRegex(ValueError, "authenticated origin mapping"):
                self.generate()

    def test_independent_runtime_catalog_must_match_subject_and_membership(self):
        catalog = {"descriptor": {"name": "syft", "version": "1.54.1"},
                   "source": {"type": "file", "metadata": {"digests": [{"algorithm": "sha256", "value": self.receipt["subject"]["sha256"]}]}},
                   "artifacts": [{"foundBy": "cargo-auditable-binary-cataloger", "metadataType": "rust-cargo-audit-entry",
                                  "metadata": self.receipt["audit"]["packages"][0]}]}
        binary_sbom.verify_syft(catalog, self.receipt)
        catalog["artifacts"] = []
        with self.assertRaisesRegex(ValueError, "runtime inventory differs"):
            binary_sbom.verify_syft(catalog, self.receipt)
        catalog["source"]["metadata"]["digests"][0]["value"] = "0" * 64
        with self.assertRaisesRegex(ValueError, "actual binary subject/tool"):
            binary_sbom.verify_syft(catalog, self.receipt)

    def test_missing_expected_root_or_invalid_reference_rejected(self):
        self.receipt["audit"]["packages"][0]["name"] = "unrelated"
        with self.assertRaisesRegex(ValueError, "application dependency metadata"):
            self.generate()
        self.receipt["audit"]["packages"][0]["name"] = "nyxid-cli"
        self.receipt["audit"]["packages"][0]["dependencies"] = [9]
        with self.assertRaisesRegex(ValueError, "dependency reference"):
            self.generate()

    def test_wizard_requires_exact_raw_seal_and_binary_subject(self):
        seal = (binary_sbom.ROOT / "cli/src/wizard/assets/shipped-composition.json").read_bytes()
        embedded = {"schema": "nyxid.embedded-wizard.v1", "subject": self.receipt["subject"],
                    "seal_sha256": hashlib.sha256(seal).hexdigest()}
        document = self.generate()
        binary_sbom.attach_wizard(document, seal, embedded)
        self.assertTrue(any(item.get("purl", "").startswith("pkg:npm/") for item in document["components"]))
        self.assertTrue(any(item["type"] == "file" and item["name"] == "index.html" for item in document["components"]))
        with self.assertRaisesRegex(ValueError, "does not bind"):
            binary_sbom.attach_wizard(self.generate(), seal + b"\n", embedded)
        embedded["subject"] = {"sha256": "0" * 64}
        with self.assertRaisesRegex(ValueError, "does not bind"):
            binary_sbom.attach_wizard(self.generate(), seal, embedded)

    def test_vendor_pedigree_is_ancestry_and_fails_if_reviewed_bytes_change(self):
        package = {"name": "proc-macro-error2", "version": "2.0.1"}
        pedigree = binary_sbom.local_pedigree(package, "a" * 40)
        self.assertEqual(pedigree["ancestors"][0]["purl"], "pkg:cargo/proc-macro-error2@2.0.1")
        self.assertEqual(pedigree["patches"][0]["resolves"][0]["type"], "defect")
        with patch.object(binary_sbom.hashlib, "sha256") as digest:
            digest.return_value.hexdigest.return_value = "0" * 64
            with self.assertRaisesRegex(ValueError, "reviewed patched source"):
                binary_sbom.local_pedigree(package, "a" * 40)


if __name__ == "__main__":
    unittest.main()
