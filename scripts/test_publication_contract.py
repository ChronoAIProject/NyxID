import copy
import io
import json
from pathlib import Path
import tempfile
import unittest

import publication_contract as contract


def dist_fixture():
    policy = contract.load_json(contract.POLICY)
    cli = policy["cli"]
    artifacts = {name: {"name": name, "kind": kind} for name, kind in cli["global_artifacts"].items()}
    for target in cli["targets"]:
        name = "nyxid-cli-" + target + ".tar.gz"
        artifacts[name] = {"name": name, "kind": "executable-zip", "target_triples": [target], "assets": [
            {"path": "nyxid", "kind": "executable"}, {"path": "LICENSE", "kind": "license"}]}
        artifacts[name + ".sha256"] = {"name": name + ".sha256", "kind": "checksum"}
    dist = {"announcement_tag": "v0.71.0", "releases": [{"app_name": "nyxid-cli", "app_version": "0.71.0", "artifacts": list(artifacts)}],
            "artifacts": artifacts, "ci": {"github": {"artifacts_matrix": {"include": [{"targets": [target]} for target in cli["targets"]]}}}}
    metadata = {"metadata": {"dist": {"targets": cli["targets"], "cargo-auditable": True}}, "packages": [{"name": "nyxid-cli", "version": "0.71.0"}]}
    return dist, metadata


def cli_plan():
    return contract.dist_contract(*dist_fixture(), "a" * 40, 1792367116)


def images_plan():
    images = contract.load_json(contract.POLICY)["images"]
    matrix = {"component": list(images["components"]), "platform": images["platforms"],
              "include": [{"component": component, "file": file} for component, file in images["components"].items()]}
    return contract.image_contract("a" * 40, 1792367116, matrix)


def records(plan):
    result = [{**item, "source": plan["source"], "digest": "sha256:" + format(index, "064x")} for index, item in enumerate(plan["subjects"], 1)]
    actual = {item["id"]: item for item in result}
    for item in result:
        if item["kind"] == "image-index":
            item["manifests"] = [{"platform": platform, "digest": actual["image:" + item["component"] + ":" + platform]["digest"]} for platform in item["platforms"]]
    return result


class PublicationContractTests(unittest.TestCase):
    def test_complete_four_target_and_five_component_two_platform_contract(self):
        cli, images = cli_plan(), images_plan()
        self.assertEqual(len(cli["subjects"]), 13)
        self.assertEqual(len(images["subjects"]), 15)
        self.assertEqual(sum(item.get("artifact_kind") == "executable-zip" for item in cli["subjects"]), 4)
        for plan in (cli, images):
            self.assertEqual(len(contract.validate_coverage(plan, records(plan))), len(plan["subjects"]))

    def test_dist_plan_and_build_matrix_cannot_reduce_expected_scope(self):
        for mutate in (lambda d: d["artifacts"].pop("nyxid-cli-x86_64-apple-darwin.tar.gz"),
                       lambda d: d["releases"][0]["artifacts"].append(d["releases"][0]["artifacts"][0]),
                       lambda d: d["artifacts"].update({"unexpected": {"kind": "installer"}}),
                       lambda d: d["ci"]["github"]["artifacts_matrix"]["include"].pop(),
                       lambda d: d["releases"][0].update(app_version="0.70.0")):
            dist, metadata = dist_fixture()
            mutate(dist)
            with self.assertRaises(ValueError):
                contract.dist_contract(dist, metadata, "a" * 40, 1792367116)

    def test_missing_duplicate_extra_and_stale_subjects_fail(self):
        for plan in (cli_plan(), images_plan()):
            good = records(plan)
            for bad in (good[:-1], good + [good[0]], good + [{**good[0], "id": "unexpected"}],
                        [{**good[0], "source": {**plan["source"], "commit": "b" * 40}}, *good[1:]]):
                with self.assertRaises(ValueError):
                    contract.validate_coverage(plan, bad)

    def test_index_platform_coverage_and_child_digest_are_exact(self):
        plan = images_plan()
        for case in ("missing", "duplicate", "extra", "digest"):
            actual = records(plan)
            item = next(item for item in actual if item["kind"] == "image-index")
            if case == "missing":
                item["manifests"].pop()
            elif case == "duplicate":
                item["manifests"].append(item["manifests"][0])
            elif case == "extra":
                item["manifests"].append({"platform": "linux/386", "digest": "sha256:" + "0" * 64})
            else:
                item["manifests"][0]["digest"] = "sha256:" + "0" * 64
            with self.assertRaises(ValueError):
                contract.validate_coverage(plan, actual)

    def test_failed_skipped_cancelled_missing_jobs_never_satisfy_publication(self):
        jobs = ["local", "global", "sign", "scan"]
        good = dict.fromkeys(jobs, "success")
        contract.require_success(good, jobs)
        for status in ("skipped", "failure", "cancelled", "pending", None):
            with self.assertRaises(ValueError):
                contract.require_success({**good, "sign": status}, jobs)
        with self.assertRaises(ValueError):
            contract.require_success({key: value for key, value in good.items() if key != "global"}, jobs)

    def test_required_sbom_manifest_signature_and_evidence_bytes_cannot_disappear(self):
        plan = cli_plan()
        actual = records(plan)
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            for index, item in enumerate(actual):
                item["evidence"] = {}
                for kind in contract.evidence_requirements(item):
                    name = str(index) + "-" + kind + ".json"
                    (directory / name).write_text(json.dumps({"structural_test_fixture": name}))
                    item["evidence"][kind] = {"path": name, "digest": contract.file_digest(directory / name)}
            contract.validate_evidence_set(plan, actual, directory)
            credential = next(item for item in actual if item["artifact_kind"] == "credential-manifest")
            signature = credential["evidence"].pop("manifest-signature")
            with self.assertRaisesRegex(ValueError, "missing/extra planned"):
                contract.validate_evidence_set(plan, actual, directory)
            credential["evidence"]["manifest-signature"] = signature
            (directory / signature["path"]).write_text("changed evidence")
            with self.assertRaisesRegex(ValueError, "stale/changed"):
                contract.validate_evidence_set(plan, actual, directory)

    def test_expected_plan_itself_cannot_omit_subjects_or_traverse_paths(self):
        plan = cli_plan()
        plan["subjects"].pop()
        with self.assertRaises(ValueError):
            contract.validate_plan(plan)
        plan = cli_plan()
        archive = next(item for item in plan["subjects"] if item["artifact_kind"] == "executable-zip")
        archive["assets"][0]["path"] = "../nyxid"
        with self.assertRaises(ValueError):
            contract.validate_plan(plan)

    def test_duplicate_json_keys_fail_before_plan_parsing(self):
        with self.assertRaisesRegex(ValueError, "duplicate JSON"):
            json.loads('{"subjects": [], "subjects": [1]}', object_pairs_hook=contract.unique_object)


if __name__ == "__main__":
    unittest.main()
