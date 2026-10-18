import importlib.util
import json
from pathlib import Path
import shutil
import tempfile
import unittest

from jsonschema import ValidationError

spec = importlib.util.spec_from_file_location("cyclonedx", Path(__file__).with_name("validate-cyclonedx.py"))
schema = importlib.util.module_from_spec(spec)
spec.loader.exec_module(schema)


class CycloneDxTests(unittest.TestCase):
    def test_official_schema_enforces_nested_types_and_fields(self):
        document = {"bomFormat": "CycloneDX", "specVersion": "1.6", "version": 1,
                    "components": [{"type": "library", "name": "control", "version": "1.0.0"}]}
        schema.validate(document)
        for value in ("invented-component-type", 123):
            document["components"][0]["type"] = value
            with self.assertRaises(ValidationError):
                schema.validate(document)
        document["components"][0]["type"] = "library"
        document["components"][0]["unrecognized"] = True
        with self.assertRaises(ValidationError):
            schema.validate(document)

    def test_referenced_license_schema_is_offline_and_enforced(self):
        document = {"bomFormat": "CycloneDX", "specVersion": "1.6", "version": 1,
                    "components": [{"type": "library", "name": "control", "licenses": [{"license": {"id": "MIT"}}]}]}
        schema.validate(document)
        document["components"][0]["licenses"][0]["license"]["id"] = "invented-license"
        with self.assertRaises(ValidationError):
            schema.validate(document)

    def test_schema_tampering_fails_before_validation(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary) / "schemas"
            shutil.copytree(schema.SCHEMAS, directory)
            path = directory / "bom-1.6.schema.json"
            path.write_text(json.dumps({}))
            with self.assertRaisesRegex(ValueError, "pinned upstream bytes"):
                schema.validator(directory)


if __name__ == "__main__":
    unittest.main()
