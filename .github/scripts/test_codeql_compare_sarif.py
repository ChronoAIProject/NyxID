import copy
import unittest

from codeql_compare_sarif import compare


def document():
    return {"runs": [{
        "tool": {"driver": {"rules": [{"id": "rust/test", "shortDescription": {"text": "Rule"}}]},
                 "extensions": [{"name": "models", "taxa": [
                     {"id": "model/a", "locations": [{"uri": "codeql/model.yml"}]},
                     {"id": "model/b"}]}]},
        "artifacts": [{"location": {"uri": "tests.rs"}}, {"location": {"uri": "src.rs"}}],
        "results": [{"ruleId": "rust/test", "ruleIndex": 0, "rule": {"id": "rust/test", "index": 0},
                     "message": {"text": "finding"},
                     "locations": [{"physicalLocation": {"artifactLocation": {"uri": "tests.rs", "index": 0}}}],
                     "codeFlows": [{"taxa": [{"index": 0, "toolComponent": {"index": 0}}]}]}],
    }]}


class ComparisonTests(unittest.TestCase):
    def test_references_are_resolved_and_flow_changes_are_not_hidden(self):
        before = document()
        after = copy.deepcopy(before)
        run = after["runs"][0]
        run["artifacts"].reverse()
        run["tool"]["extensions"][0]["taxa"].reverse()
        result = run["results"][0]
        result["locations"][0]["physicalLocation"]["artifactLocation"]["index"] = 1
        result["codeFlows"][0]["taxa"][0]["index"] = 1
        self.assertEqual(compare(before, after, set())["unchanged"], 1)
        result["codeFlows"][0]["taxa"][0]["index"] = 0
        self.assertEqual(len(compare(before, after, set())["added_or_changed"]), 1)

    def test_removed_alerts_must_have_only_excluded_locations(self):
        before = document()
        after = copy.deepcopy(before)
        after["runs"][0]["results"] = []
        self.assertEqual(compare(before, after, {"tests.rs"})["removed_outside_exclusions"], [])
        self.assertEqual(len(compare(before, after, set())["removed_outside_exclusions"]), 1)
        before["runs"][0]["results"][0]["relatedLocations"] = [{
            "physicalLocation": {"artifactLocation": {"uri": "src.rs", "index": 1}}}]
        self.assertEqual(len(compare(before, after, {"tests.rs"})["removed_outside_exclusions"]), 1)


if __name__ == "__main__":
    unittest.main()
