import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("embedded", Path(__file__).with_name("verify-embedded-wizard.py"))
embedded = importlib.util.module_from_spec(spec)
spec.loader.exec_module(embedded)


class EmbeddedWizardTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.assets = self.root / "assets"
        self.assets.mkdir()
        self.html = b"<html>public wizard fixture</html>"
        (self.assets / "index.html").write_bytes(self.html)
        self.seal = json.dumps({"schema": "nyxid.shipped-composition.v1", "component": "wizard", "artifacts": [
            {"path": "index.html", "size": len(self.html), "sha256": hashlib.sha256(self.html).hexdigest()}]}).encode()
        (self.assets / "shipped-composition.json").write_bytes(self.seal)
        self.binary = self.root / "nyxid"
        self.binary.write_bytes(b"binary-header" + self.html + b"padding" + self.seal)

    def test_binds_actual_subject_and_each_asset(self):
        receipt = embedded.verify(self.binary, self.assets)
        self.assertEqual(receipt["subject"]["sha256"], hashlib.sha256(self.binary.read_bytes()).hexdigest())
        self.assertEqual(len(receipt["assets"]), 2)

    def test_missing_or_altered_final_binary_bytes_fail(self):
        for data in (self.html, self.seal, b"binary without wizard"):
            self.binary.write_bytes(data)
            with self.assertRaisesRegex(ValueError, "absent from final binary"):
                embedded.verify(self.binary, self.assets)

    def test_stale_asset_or_unsealed_file_fails(self):
        (self.assets / "index.html").write_bytes(self.html + b"changed")
        with self.assertRaisesRegex(ValueError, "differs from final-byte seal"):
            embedded.verify(self.binary, self.assets)
        (self.assets / "extra.js").write_bytes(b"unsealed")
        with self.assertRaisesRegex(ValueError, "asset set differs"):
            embedded.verify(self.binary, self.assets)


if __name__ == "__main__":
    unittest.main()
