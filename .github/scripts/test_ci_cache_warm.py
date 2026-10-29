"""Keep ordinary CI readers aligned with the sole main cache writer."""
import fnmatch
import itertools
import json
import re
import unittest
from pathlib import Path

from ci_gate_reuse import strict_load_workflow


CACHE_ACTION = "./.github/actions/ci-rust-cache"


def load(name):
    return strict_load_workflow(Path(name).read_text())


def render(value, variables):
    if not isinstance(value, str):
        return value
    for name, replacement in variables.items():
        value = re.sub(r"\$\{\{\s*" + re.escape(name) + r"\s*\}\}", str(replacement), value)
    if "inputs.coverage &&" in value:
        value = re.sub(r"\$\{\{ inputs.coverage && '([^']*)' \|\| '([^']*)' \}\}",
                       lambda m: m[1] if variables["inputs.coverage"] else m[2], value)
    return value


def cache_lanes(workflow, action, coverage=None):
    lanes = []
    for job_id, job in workflow["jobs"].items():
        matrix = job.get("strategy", {}).get("matrix", {})
        rows = [dict(zip(matrix, row)) for row in itertools.product(*matrix.values())]
        for row in rows:
            variables = {f"matrix.{key}": value for key, value in row.items()}
            variables["inputs.coverage"] = coverage
            for step in job.get("steps", []):
                if step.get("uses") != CACHE_ACTION:
                    continue
                inputs = {key: render(value, variables) for key, value in step["with"].items()}
                env = {**workflow.get("env", {}), **job.get("env", {}), **step.get("env", {})}
                # Swatinem's default env-vars are CARGO CC CFLAGS CXX CMAKE RUST.
                # The composite pins scheduling-only variables for this step.
                rust_step = next(s for s in action["runs"]["steps"]
                                 if s.get("uses", "").startswith("Swatinem/rust-cache@"))
                env.update(rust_step.get("env", {}))
                hashed_env = {k: str(v) for k, v in env.items()
                              if k.startswith(("CARGO", "CC", "CFLAGS", "CXX", "CMAKE", "RUST"))}
                toolchain = next(s for s in job["steps"]
                                 if s.get("uses", "").startswith("dtolnay/rust-toolchain@"))
                signature = (job["runs-on"], toolchain["uses"],
                             inputs.get("incremental", "false"), hashed_env)
                lanes.append((inputs["shared-key"], signature, inputs, job, variables, job_id))
    return lanes


class CacheWarmTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.ci = load(".github/workflows/ci.yml")
        cls.backend = load(".github/workflows/backend-tests.yml")
        cls.warm = load(".github/workflows/ci-cache-warm.yml")
        cls.action = load(".github/actions/ci-rust-cache/action.yml")

    def test_warm_is_the_only_ordinary_rust_writer(self):
        self.assertEqual(self.action["inputs"]["save-if"]["default"], "false")
        for path in Path(".github/workflows").glob("*.yml"):
            workflow = load(str(path))
            writer = path.name == "ci-cache-warm.yml"
            for job in workflow["jobs"].values():
                for step in job.get("steps", []):
                    if step.get("uses") == CACHE_ACTION:
                        self.assertEqual(step["with"]["save-if"], "true" if writer else "false")
                    if (step.get("uses", "").startswith("actions/cache/save@")
                            and str(step.get("with", {}).get("key", "")).startswith("v0-rust-incremental-")):
                        self.assertTrue(writer, path)
                        self.assertEqual(step["if"], "github.ref == 'refs/heads/main' && success()")

    def test_every_reader_has_one_writer_with_identical_key_inputs(self):
        writers = cache_lanes(self.warm, self.action)
        keys = [lane[0] for lane in writers]
        self.assertEqual(len(keys), len(set(keys)))
        self.assertEqual(set(keys), {
            "backend-build", "coverage-backend-build", "rust-clippy",
            "features-aws-kms-rust-features", "features-gcp-kms-rust-features",
            "features-aws-and-gcp-kms-rust-features", "cli-cli-test",
            "coverage-cli-coverage-cli", "permissions-test",
            "backend-image-inputs-backend-image-inputs",
        })
        readers = cache_lanes(self.ci, self.action)
        readers += cache_lanes(self.backend, self.action, False)
        readers += cache_lanes(self.backend, self.action, True)
        by_key = {lane[0]: lane for lane in writers}
        for key, signature, _, _, _, job_id in readers:
            with self.subTest(lane=key, job=job_id):
                self.assertEqual(signature, by_key[key][1])
        self.assertEqual(set(key for key, *_ in readers), set(keys))

    def test_incremental_save_keys_and_paths_match_composite_restore(self):
        restore = next(s for s in self.action["runs"]["steps"]
                       if s.get("uses") == "actions/cache/restore@v4")
        for lane, _, inputs, job, variables, _ in cache_lanes(self.warm, self.action):
            if inputs.get("incremental") != "true":
                continue
            saves = [s for s in job["steps"] if s.get("uses") == "actions/cache/save@v4"]
            self.assertEqual(len(saves), 1, lane)
            expected = render(restore["with"]["key"], {"inputs.shared-key": lane})
            self.assertEqual(render(saves[0]["with"]["key"], variables), expected)
            paths = set(saves[0]["with"]["path"].split())
            self.assertTrue(paths <= set(restore["with"]["path"].split()))
            self.assertIn("target/debug/incremental", paths)
            if lane.startswith("coverage-"):
                self.assertIn("target/llvm-cov-target/debug/incremental", paths)

    def test_warm_runs_on_backend_and_all_workspace_main_changes(self):
        trigger = self.warm["on"]["push"]
        self.assertEqual(trigger["branches"], ["main"])
        self.assertIn("workflow_dispatch", self.warm["on"])
        members = re.search(r'^members = (\[.*\])$', Path("Cargo.toml").read_text(), re.M)[1]
        paths = [f"{member}/src/lib.rs" for member in json.loads(members)]
        paths += ["backend/src/handlers/nyxbot.rs", "Cargo.lock", "Cargo.toml",
                  "rust-toolchain.toml", ".cargo/config.toml", ".config/nextest.toml",
                  ".github/workflows/ci.yml", ".github/actions/ci-rust-cache/action.yml",
                  ".github/scripts/retry_incremental.py", "docs/AI_AGENT_PLAYBOOK.md",
                  "skills/nyxid/scripts/nyxid.py", "tests/fixtures/assistant/readiness-v1.json",
                  "docs/assistant/CHANNEL_SETUP_GUIDE.md",
                  "integrations/oracle/cdp-worker/worker.mjs"]
        for path in paths:
            self.assertTrue(any(fnmatch.fnmatchcase(path, glob) for glob in trigger["paths"]), path)
        self.assertEqual(self.warm["concurrency"], {"group": "ci-cache-warm", "cancel-in-progress": True})
        self.assertTrue(all(job.get("continue-on-error") for job in self.warm["jobs"].values()))

    def test_feature_variants_keep_flags_and_lane_names(self):
        expected = {"aws-kms": "aws-kms", "gcp-kms": "gcp-kms", "aws-and-gcp-kms": "aws-kms,gcp-kms"}
        for workflow in (self.ci, self.warm):
            job = workflow["jobs"]["rust-features"]
            self.assertEqual(job["strategy"]["matrix"], {"variant": list(expected)})
            step = next(s for s in job["steps"] if s.get("name") == "Build feature set")
            expression = step["env"]["FEATURES"]
            mapping = re.fullmatch(r"\$\{\{ fromJSON\('([^']+)'\)\[matrix.variant\] \}\}", expression)
            self.assertIsNotNone(mapping)
            self.assertEqual(json.loads(mapping[1]), expected)
            self.assertIn('cargo build -p nyxid --features "$FEATURES"', step["run"])
            cache = next(s for s in job["steps"] if s.get("uses") == CACHE_ACTION)
            self.assertNotIn("FEATURES", job.get("env", {}))
            self.assertNotIn("FEATURES", cache.get("env", {}))

    def test_cli_coverage_threshold_and_pr_artifact_contract_are_preserved(self):
        jobs = self.ci["jobs"]
        head = jobs["coverage-cli"]
        commands = "\n".join(s.get("run", "") for s in head["steps"])
        self.assertIn("cargo llvm-cov --package nyxid-cli --no-report", commands)
        self.assertIn("cargo llvm-cov report --package nyxid-cli --fail-under-lines 64", commands)
        self.assertIn("coverage-cli", jobs["ci-pass"]["needs"])
        for name in ("coverage-cli-base", "coverage-cli-summary"):
            self.assertIn("github.event_name == 'pull_request'", jobs[name]["if"])
            self.assertNotIn(name, jobs["ci-pass"]["needs"])
        base = jobs["coverage-cli-base"]
        self.assertEqual(base["needs"], "changes")  # Still parallel with the head measurement.
        self.assertTrue(base["continue-on-error"])
        measure = next(s for s in base["steps"] if s.get("name") == "Measure base branch")
        self.assertEqual(measure["env"]["BASE_REF"], "${{ github.event.pull_request.base.sha }}")
        self.assertIn("cargo llvm-cov --package nyxid-cli --no-report", measure["run"])
        summary = jobs["coverage-cli-summary"]
        downloads = [s["with"]["name"] for s in summary["steps"]
                     if s.get("uses") == "actions/download-artifact@v4"]
        self.assertEqual(downloads, ["cli-coverage-head", "cli-base-report"])
        upload = next(s for s in summary["steps"] if s.get("uses") == "actions/upload-artifact@v4")
        self.assertEqual(upload["with"]["name"], "coverage-cli")
        self.assertEqual(set(upload["with"]["path"].split()), {"coverage-cli.json", "coverage-cli.lcov"})
        self.assertIn("coverage-cli-summary", jobs["coverage-comment"]["needs"])


if __name__ == "__main__":
    unittest.main()
