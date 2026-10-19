import importlib.util
from pathlib import Path
import unittest
from unittest.mock import patch


spec = importlib.util.spec_from_file_location(
    "catalog_spec_drift", Path(__file__).with_name("check-catalog-spec-drift.py")
)
drift = importlib.util.module_from_spec(spec)
spec.loader.exec_module(drift)


class NotionDriftTests(unittest.TestCase):
    def setUp(self):
        self.query = ("POST", "/v1/databases/{database_id}/query")
        self.overlay = {
            "info": {"version": "2022-06-28-nyxid-overlay"},
            "paths": {self.query[1]: {"post": {}}, "/v1/search": {"post": {}}},
        }
        self.upstream = {"paths": {"/v1/search": {"post": {}}}}
        self.reference = "versions up to and including `2022-06-28`\nnotion.databases.query({"

    def test_notion_has_an_official_spec(self):
        self.assertEqual(drift.OFFICIAL_SPECS["notion.openapi.json"][0], "https://developers.notion.com/openapi.json")

    def test_legacy_query_requires_live_reference_and_exact_version(self):
        with patch.object(drift, "fetch_text", return_value=self.reference) as fetch:
            self.assertEqual(drift.missing_operations("notion.openapi.json", self.overlay, self.upstream), set())
            fetch.assert_called_once()
            self.overlay["info"]["version"] = "2025-09-03-nyxid-overlay"
            self.assertEqual(drift.missing_operations("notion.openapi.json", self.overlay, self.upstream), {self.query})

    def test_removed_current_operation_is_not_hidden_by_legacy_query(self):
        with patch.object(drift, "fetch_text", return_value=self.reference):
            self.assertEqual(drift.missing_operations("notion.openapi.json", self.overlay, {"paths": {}}), {("POST", "/v1/search")})

    def test_changed_legacy_reference_is_drift(self):
        with patch.object(drift, "fetch_text", return_value="This API has been removed"):
            self.assertEqual(drift.missing_operations("notion.openapi.json", self.overlay, self.upstream), {self.query})

    def test_unreachable_reference_cannot_pass(self):
        with patch.object(drift, "fetch_text", side_effect=OSError("unreachable")):
            with self.assertRaises(OSError):
                drift.missing_operations("notion.openapi.json", self.overlay, self.upstream)


if __name__ == "__main__":
    unittest.main()
