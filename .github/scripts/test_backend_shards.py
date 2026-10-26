import json
import os
from pathlib import Path
import tempfile
import unittest
import xml.etree.ElementTree as ET

from backend_shards import stage_profiles, verify


class BackendShardTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.inventory = self.root / "inventory.json"
        self.results = self.root / "results"
        self.output = self.root / "combined.xml"
        cases = {name: {"ignored": False, "filter-match": {"status": "matches"}}
                 for name in ("first", "second", "third")}
        cases["ignored"] = {"ignored": True, "filter-match": {
            "status": "mismatch", "reason": "ignored"}}
        self.inventory.write_text(json.dumps({"rust-suites": {
            "nyxid::bin/nyxid-server": {"status": "listed", "testcases": cases}}}))
        for index, name in enumerate(("first", "second", "third"), 1):
            self.report(index, name)

    def report(self, shard, name, child=None):
        path = self.results / f"shard-{shard}" / "junit.xml"
        path.parent.mkdir(parents=True, exist_ok=True)
        root = ET.Element("testsuites")
        suite = ET.SubElement(root, "testsuite", name="nyxid::bin/nyxid-server")
        case = ET.SubElement(suite, "testcase", classname="nyxid::bin/nyxid-server", name=name)
        if child:
            ET.SubElement(case, child)
        ET.ElementTree(root).write(path)

    def check(self):
        verify(self.inventory, self.results, 3, self.output)

    def test_complete_partition_preserves_all_cases_and_ignored_inventory(self):
        self.check()
        root = ET.parse(self.output).getroot()
        self.assertEqual(root.attrib["tests"], "3")
        self.assertEqual(len(root.findall(".//testcase")), 3)

    def test_missing_shard_cannot_pass(self):
        (self.results / "shard-2/junit.xml").unlink()
        with self.assertRaises(FileNotFoundError):
            self.check()

    def test_unexpected_shard_directory_cannot_be_ignored(self):
        (self.results / "shard-4").mkdir()
        with self.assertRaisesRegex(ValueError, "shard directory mismatch"):
            self.check()

    def test_mismatched_shard_count_cannot_pass(self):
        for count in (0, 2, 4):
            with self.subTest(count=count):
                with self.assertRaisesRegex(ValueError, "shard directory mismatch"):
                    verify(self.inventory, self.results, count, self.output)
                with self.assertRaisesRegex(ValueError, "shard directory mismatch"):
                    stage_profiles(self.results, count, self.root / "profiles")

    def test_duplicate_partition_cannot_hide_a_missing_test(self):
        self.report(3, "first")
        with self.assertRaisesRegex(ValueError, "more than once"):
            self.check()

    def test_unexpected_test_cannot_replace_an_expected_test(self):
        self.report(3, "unrelated")
        with self.assertRaisesRegex(ValueError, "inventory mismatch"):
            self.check()

    def test_failure_skip_and_error_cannot_pass(self):
        for child in ("failure", "error", "skipped", "rerunFailure"):
            with self.subTest(child=child):
                self.report(3, "third", child)
                with self.assertRaisesRegex(ValueError, "non-passing test"):
                    self.check()

    def test_retried_then_passed_tests_are_accepted_and_reported(self):
        for child in ("flakyFailure", "flakyError"):
            with self.subTest(child=child):
                self.report(3, "third", child)
                summary = self.root / "summary.md"
                summary.write_text("")
                os.environ["GITHUB_STEP_SUMMARY"] = str(summary)
                try:
                    flaky = verify(self.inventory, self.results, 3, self.output)
                finally:
                    del os.environ["GITHUB_STEP_SUMMARY"]
                self.assertEqual(flaky, [(3, ("nyxid::bin/nyxid-server", "third"))])
                self.assertIn("third", summary.read_text())
                self.assertIn("passed only on retry", summary.read_text())

    def test_empty_report_cannot_pass(self):
        (self.results / "shard-2/junit.xml").write_text("<testsuites/>")
        with self.assertRaisesRegex(ValueError, "empty shard"):
            self.check()

    def test_inventory_exclusions_cannot_silently_drop_tests(self):
        inventory = json.loads(self.inventory.read_text())
        cases = inventory["rust-suites"]["nyxid::bin/nyxid-server"]["testcases"]
        cases["second"]["filter-match"] = {"status": "mismatch", "reason": "expression"}
        self.inventory.write_text(json.dumps(inventory))
        with self.assertRaisesRegex(ValueError, "unexpected inventory exclusion"):
            self.check()

    def test_empty_or_unlisted_inventory_cannot_pass(self):
        original = json.loads(self.inventory.read_text())
        self.inventory.write_text(json.dumps({"rust-suites": {}}))
        with self.assertRaisesRegex(ValueError, "inventory is empty"):
            self.check()
        original["rust-suites"]["nyxid::bin/nyxid-server"]["status"] = "skipped"
        self.inventory.write_text(json.dumps(original))
        with self.assertRaisesRegex(ValueError, "binary was not listed"):
            self.check()

    def test_test_names_are_scoped_to_their_binary(self):
        inventory = json.loads(self.inventory.read_text())
        inventory["rust-suites"]["nyxid-service-adapters"] = {
            "status": "listed", "testcases": {
                "first": {"ignored": False, "filter-match": {"status": "matches"}}}}
        self.inventory.write_text(json.dumps(inventory))
        path = self.results / "shard-1/junit.xml"
        root = ET.parse(path).getroot()
        suite = ET.SubElement(root, "testsuite", name="nyxid-service-adapters")
        ET.SubElement(suite, "testcase", classname="nyxid-service-adapters", name="first")
        ET.ElementTree(root).write(path)
        self.check()
        self.assertEqual(ET.parse(self.output).getroot().attrib["tests"], "4")

    def test_identical_profile_filenames_are_preserved_for_every_shard(self):
        for shard in range(1, 4):
            (self.results / f"shard-{shard}/nyxid-0.profraw").write_bytes(bytes([shard]))
        destination = self.root / "profiles"
        stage_profiles(self.results, 3, destination)
        self.assertEqual(sorted(p.read_bytes() for p in destination.iterdir()),
                         [b"\x01", b"\x02", b"\x03"])
        with self.assertRaisesRegex(ValueError, "already contains"):
            stage_profiles(self.results, 3, destination)

    def test_missing_or_empty_profiles_cannot_produce_partial_coverage(self):
        for shard in range(1, 3):
            (self.results / f"shard-{shard}/nyxid-0.profraw").write_bytes(b"profile")
        destination = self.root / "profiles"
        for empty in (False, True):
            if empty:
                (self.results / "shard-3/nyxid-0.profraw").touch()
            with self.assertRaisesRegex(ValueError, "missing or empty"):
                stage_profiles(self.results, 3, destination)
            self.assertFalse(destination.exists())


if __name__ == "__main__":
    unittest.main()
