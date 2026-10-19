import base64
import copy
import hashlib
import json
from pathlib import Path
import unittest

import oci_subjects as oci


class OciSubjectTests(unittest.TestCase):
    def setUp(self):
        capture = json.loads((Path(__file__).parent / "fixtures/oci/buildkit-alpine-fixture.json").read_text())
        self.blobs = {"sha256:" + key.split("/")[-1]: base64.b64decode(value["base64"]) for key, value in capture["records"].items() if key.startswith("blobs/")}
        index = json.loads(base64.b64decode(capture["records"]["index.json"]["base64"]))
        self.root = index["manifests"][0]
        self.envelope = json.loads(self.blobs[self.root["digest"]])

    def put(self, document):
        data = json.dumps(document, separators=(",", ":")).encode()
        digest = "sha256:" + hashlib.sha256(data).hexdigest()
        self.blobs[digest] = data
        return {"mediaType": document.get("mediaType", "application/vnd.in-toto+json"), "digest": digest, "size": len(data)}

    def inspect(self, root=None, platforms=None, commit=None):
        return oci.inspect(root or self.root, self.blobs.__getitem__, platforms or ["linux/arm64"], commit)

    def test_real_buildkit_index_preserves_runnable_and_bound_auxiliary(self):
        result = self.inspect()
        self.assertEqual(result["subject"]["digest"], self.root["digest"])
        self.assertEqual(len(result["runnable"]), 1)
        self.assertEqual(len(result["auxiliaries"]), 1)
        self.assertEqual(result["auxiliaries"][0]["reference"], result["runnable"][0]["digest"])
        self.assertEqual(result["runnable"][0]["platform"], "linux/arm64")

    def test_complete_expected_runnable_platform_set_is_required(self):
        with self.assertRaisesRegex(ValueError, "missing planned"):
            self.inspect(platforms=["linux/arm64", "linux/amd64"])
        with self.assertRaisesRegex(ValueError, "mismatched OCI runnable platform"):
            self.inspect(platforms=["linux/amd64"])

    def test_explicit_supported_arm64_variant_is_preserved_by_policy(self):
        envelope = copy.deepcopy(self.envelope)
        envelope["manifests"][0]["platform"]["variant"] = "v8"
        self.assertEqual(self.inspect(self.put(envelope))["runnable"][0]["platform"], "linux/arm64")
        for field, value in (("variant", "v9"), ("os.version", "unexpected")):
            bad = copy.deepcopy(envelope)
            bad["manifests"][0]["platform"][field] = value
            with self.assertRaisesRegex(ValueError, "mismatched OCI runnable platform"):
                self.inspect(self.put(bad))

    def test_nested_envelope_preserves_every_ancestor_platform_boundary(self):
        for platform, valid in (({"os": "linux", "architecture": "arm64", "variant": "v8"}, True),
                                ({"os": "linux", "architecture": "amd64"}, False)):
            outer = {"schemaVersion": 2, "mediaType": oci.OCI_INDEX,
                     "manifests": [{**self.root, "platform": platform}]}
            if valid:
                self.assertEqual(self.inspect(self.put(outer))["runnable"][0]["platform"], "linux/arm64")
            else:
                with self.assertRaisesRegex(ValueError, "mismatched OCI runnable platform"):
                    self.inspect(self.put(outer))

    def test_direct_manifest_config_variant_is_validated_without_descriptor_platform(self):
        original = self.envelope["manifests"][0]
        for variant in (None, "v8", "v9"):
            manifest = json.loads(self.blobs[original["digest"]])
            config = json.loads(self.blobs[manifest["config"]["digest"]])
            if variant is None:
                config.pop("variant", None)
            else:
                config["variant"] = variant
            manifest["config"] = {**self.put(config), "mediaType": "application/vnd.oci.image.config.v1+json"}
            descriptor = self.put(manifest)
            if variant != "v9":
                self.assertEqual(self.inspect(descriptor)["runnable"][0]["platform"], "linux/arm64")
            else:
                with self.assertRaisesRegex(ValueError, "mismatched OCI runnable platform"):
                    self.inspect(descriptor)

    def test_duplicate_runnable_and_unbound_or_unknown_auxiliaries_fail(self):
        envelope = copy.deepcopy(self.envelope)
        envelope["manifests"].append(envelope["manifests"][0])
        with self.assertRaisesRegex(ValueError, "duplicate OCI subject"):
            self.inspect(self.put(envelope))
        envelope = copy.deepcopy(self.envelope)
        envelope["manifests"][1]["annotations"]["vnd.docker.reference.digest"] = "sha256:" + "0" * 64
        with self.assertRaisesRegex(ValueError, "unbound/duplicate/unsupported"):
            self.inspect(self.put(envelope))
        envelope = copy.deepcopy(self.envelope)
        envelope["manifests"][1]["annotations"]["vnd.docker.reference.type"] = "unknown-auxiliary"
        with self.assertRaisesRegex(ValueError, "mismatched OCI runnable platform"):
            self.inspect(self.put(envelope))

    def test_well_framed_auxiliary_statement_cannot_name_different_subject(self):
        envelope = copy.deepcopy(self.envelope)
        descriptor = envelope["manifests"][1]
        manifest = json.loads(self.blobs[descriptor["digest"]])
        layer = manifest["layers"][0]
        statement = json.loads(self.blobs[layer["digest"]])
        statement["subject"][0]["digest"]["sha256"] = "0" * 64
        manifest["layers"][0] = {**layer, **self.put(statement)}
        config = json.loads(self.blobs[manifest["config"]["digest"]])
        config["rootfs"]["diff_ids"] = [manifest["layers"][0]["digest"]]
        manifest["config"] = {**manifest["config"], **self.put(config), "mediaType": "application/vnd.oci.image.config.v1+json"}
        envelope["manifests"][1] = {**descriptor, **self.put(manifest)}
        with self.assertRaisesRegex(ValueError, "statement differs from referenced"):
            self.inspect(self.put(envelope))

    def test_metadata_hash_size_and_source_revision_are_checked(self):
        with self.assertRaisesRegex(ValueError, "source revision"):
            self.inspect(commit="a" * 40)
        with self.assertRaisesRegex(ValueError, "content descriptor"):
            self.inspect({**self.root, "size": self.root["size"] + 1})
        self.blobs[self.root["digest"]] += b" "
        with self.assertRaisesRegex(ValueError, "content descriptor"):
            self.inspect()


if __name__ == "__main__":
    unittest.main()
