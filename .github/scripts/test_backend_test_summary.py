from pathlib import Path
import tempfile
import unittest
import xml.etree.ElementTree as ET

from backend_test_summary import MAX_SUMMARY_BYTES, markdown, summarize


class BackendTestSummaryTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)

    def report(self, shard, cases):
        path = self.root / f"shard-{shard}" / "junit.xml"
        path.parent.mkdir(parents=True, exist_ok=True)
        root = ET.Element("testsuites")
        suite = ET.SubElement(root, "testsuite")
        for name, seconds, tags in cases:
            case = ET.SubElement(suite, "testcase", classname="backend", name=name, time=seconds)
            for tag in tags:
                ET.SubElement(case, tag)
        ET.ElementTree(root).write(path)

    def test_failure_does_not_hide_retries_or_slow_tests_on_other_shards(self):
        self.report(1, [("failure", "2", ["failure"]), ("retry", "1", ["flakyFailure"])])
        self.report(2, [("slow", "10", []), ("skip", "0", ["skipped"])])
        report = summarize(self.root, 2)
        self.assertEqual(report["counts"], {"passed": 1, "retried": 1, "failed": 1, "skipped": 1})
        self.assertEqual(report["slowest"][0]["test"], "slow")
        self.assertIn("retry", markdown(report))
        self.assertIn("failure", markdown(report))

    def test_missing_and_malformed_shards_preserve_available_evidence(self):
        self.report(1, [("known-failure", "1", ["error"])])
        self.report(2, [])
        (self.root / "shard-2/junit.xml").write_text("<broken")
        report = summarize(self.root, 3)
        self.assertEqual([item["shard"] for item in report["unavailable"]], [2, 3])
        self.assertEqual(report["counts"]["failed"], 1)

    def test_final_failure_takes_precedence_over_a_previous_attempt(self):
        for tag in ("failure", "error", "rerunFailure"):
            with self.subTest(tag=tag):
                self.report(1, [("still-fails", "1", ["flakyFailure", tag])])
                report = summarize(self.root, 1)
                self.assertEqual(report["counts"]["failed"], 1)
                self.assertEqual(report["counts"]["retried"], 0)

    def test_empty_report_is_explicitly_incomplete(self):
        self.report(1, [])
        report = summarize(self.root, 1)
        self.assertEqual(report["unavailable"], [{"shard": 1, "reason": "empty test report"}])
        self.assertEqual(report["counts"]["passed"], 0)

    def test_invalid_times_and_markup_are_safe_to_render(self):
        self.report(1, [("<tag>`", "NaN", []), ("bad", "unknown", []), ("negative", "-1", [])])
        report = summarize(self.root, 1)
        self.assertTrue(all(case["seconds"] == 0 for case in report["slowest"]))
        self.assertIn("&lt;tag&gt;&#96;", markdown(report))
        self.assertNotIn("<tag>", markdown(report))

    def test_slowest_list_is_bounded_without_truncating_failures(self):
        self.report(1, [(f"case-{i}", str(i), ["failure"]) for i in range(25)])
        report = summarize(self.root, 1)
        self.assertEqual(len(report["slowest"]), 20)
        self.assertEqual(len(report["attention"]), 25)
        self.assertIn("case-24", markdown(report))

    def test_mass_failure_bounds_display_but_preserves_complete_evidence(self):
        cases = [(f"case-{i}-" + "雪<>&`" * 100, "1", ["failure"])
                 for i in range(1000)]
        self.report(1, cases)
        report = summarize(self.root, 1)
        rendered = markdown(report)
        self.assertLessEqual(len(rendered.encode("utf-8")), MAX_SUMMARY_BYTES)
        self.assertIn("Summary truncated", rendered)
        self.assertIn("JSON and JUnit artifacts", rendered)
        self.assertEqual(rendered.count("<code>"), rendered.count("</code>"))
        self.assertNotIn("\ufffd", rendered)
        self.assertEqual(report["counts"]["failed"], len(cases))
        self.assertEqual([case["test"] for case in report["attention"]],
                         [case[0] for case in cases])


if __name__ == "__main__":
    unittest.main()
