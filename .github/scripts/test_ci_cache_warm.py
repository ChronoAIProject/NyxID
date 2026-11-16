"""Keep ordinary CI readers aligned with the sole cache writer."""
import fnmatch
import itertools
import json
import os
import re
import subprocess
import tempfile
import unittest
from pathlib import Path

from ci_gate_reuse import strict_load_workflow
from incremental_cache import ROOTS as INCREMENTAL_ROOTS


CACHE_ACTION = "./.github/actions/ci-rust-cache"
SAVE_ACTION = "./.github/actions/ci-rust-incremental-save"
INCREMENTAL_PATHS = (
    "target/debug/incremental",
    "target/llvm-cov-target/debug/incremental",
)


def load(name):
    return strict_load_workflow(Path(name).read_text())


def github_output(text):
    values = {}
    lines = text.splitlines()
    index = 0
    while index < len(lines):
        line = lines[index]
        if line.endswith("<<EOF"):
            name = line[: -len("<<EOF")]
            index += 1
            body = []
            while index < len(lines) and lines[index] != "EOF":
                body.append(lines[index])
                index += 1
            values[name] = "\n".join(body)
        elif "=" in line:
            name, value = line.split("=", 1)
            values[name] = value
        index += 1
    return values


def run_bash(script, env):
    with tempfile.TemporaryDirectory() as tmp:
        output = Path(tmp) / "github-output"
        full = os.environ.copy()
        full.update(env)
        full["GITHUB_OUTPUT"] = str(output)
        subprocess.run(["bash", "-c", script], check=True, env=full)
        return github_output(output.read_text())


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
        cls.save = load(".github/actions/ci-rust-incremental-save/action.yml")
        cls.action_text = Path(".github/actions/ci-rust-cache/action.yml").read_text()
        cls.warm_text = Path(".github/workflows/ci-cache-warm.yml").read_text()

    def test_warm_is_the_only_ordinary_rust_writer(self):
        self.assertEqual(self.action["inputs"]["save-if"]["default"], "false")
        for path in Path(".github/workflows").glob("*.yml"):
            workflow = load(str(path))
            writer = path.name == "ci-cache-warm.yml"
            for job in workflow["jobs"].values():
                for step in job.get("steps", []):
                    if step.get("uses") == CACHE_ACTION:
                        self.assertEqual(step["with"]["save-if"], "true" if writer else "false")
                    if step.get("uses") == SAVE_ACTION:
                        self.assertTrue(writer, path)
                    key = str(step.get("with", {}).get("key", ""))
                    paths = str(step.get("with", {}).get("path", ""))
                    if (step.get("uses", "").startswith("actions/cache/save@")
                            and (writer or "rust-incremental" in key or "/incremental" in paths)):
                        self.fail(f"{path} writes an incremental cache outside the shared contract")

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

    def test_incremental_key_and_paths_have_one_contract(self):
        self.assertEqual(tuple(map(str, INCREMENTAL_ROOTS)), INCREMENTAL_PATHS)
        self.assertEqual(self.action["outputs"]["incremental-key"]["value"],
                         "${{ steps.incremental-contract.outputs.key }}")
        self.assertEqual(self.action["outputs"]["incremental-path"]["value"],
                         "${{ steps.incremental-contract.outputs.path }}")
        self.assertEqual(self.action_text.count("v1-rust-graph-"), 1)
        definition = "printf '%s\\n' " + " ".join(INCREMENTAL_PATHS)
        self.assertEqual(self.action_text.count(definition), 1)
        save_text = Path(".github/actions/ci-rust-incremental-save/action.yml").read_text()
        for path in INCREMENTAL_PATHS:
            self.assertNotIn(path, self.warm_text)
            self.assertNotIn(path, save_text)
        contract = next(step for step in self.action["runs"]["steps"]
                        if step.get("id") == "incremental-contract")
        key_expression = contract["env"]["INCREMENTAL_KEY"]
        self.assertIn("${{ inputs.shared-key }}", key_expression)
        self.assertIn("${{ runner.os }}", key_expression)
        self.assertIn("${{ runner.arch }}", key_expression)
        self.assertIn("${{ github.sha }}", key_expression)
        self.assertIn(
            "hashFiles('**/Cargo.toml', '**/Cargo.lock', 'rust-toolchain*', '.cargo/config.toml')",
            key_expression,
        )
        restore = next(step for step in self.action["runs"]["steps"]
                       if step.get("uses") == "actions/cache/restore@v4")
        self.assertEqual(restore["with"], {
            "path": "${{ steps.incremental-contract.outputs.path }}",
            "key": "${{ steps.incremental-contract.outputs.key }}",
            "restore-keys": "${{ steps.incremental-contract.outputs.restore-keys }}",
        })
        save = next(step for step in self.save["runs"]["steps"]
                    if step.get("uses") == "actions/cache/save@v4")
        self.assertEqual(save["with"], {"path": "${{ inputs.path }}", "key": "${{ inputs.key }}"})
        sample = (
            "v1-rust-graph-backend-build-Linux-X64-"
            + "ab" * 32 + "-" + "cd" * 20
        )
        emitted = run_bash(contract["run"], {"INCREMENTAL_KEY": sample})
        self.assertEqual(emitted["key"], sample)
        self.assertEqual(emitted["path"].splitlines(), list(INCREMENTAL_PATHS))
        manifest, broad = emitted["restore-keys"].splitlines()
        self.assertEqual(manifest, sample.removesuffix("cd" * 20))
        self.assertEqual(broad, "v1-rust-graph-backend-build-Linux-X64-")
        self.assertNotEqual(manifest, broad)
        for lane, _, inputs, job, _, _ in cache_lanes(self.warm, self.action):
            saves = [step for step in job["steps"] if step.get("uses") == SAVE_ACTION]
            cache = next(step for step in job["steps"] if step.get("uses") == CACHE_ACTION)
            if inputs.get("incremental") != "true":
                self.assertEqual(saves, [], lane)
                continue
            self.assertEqual(cache.get("id"), "rust-cache", lane)
            self.assertEqual(len(saves), 1, lane)
            self.assertEqual(saves[0]["if"], "success() && steps.rust-cache.outputs.save-enabled == 'true'")
            self.assertEqual(saves[0]["with"], {
                "key": "${{ steps.rust-cache.outputs.incremental-key }}",
                "path": "${{ steps.rust-cache.outputs.incremental-path }}",
            })

    def test_archive_policy_and_remaining_graph_producers(self):
        for coverage in (False, True):
            lanes = cache_lanes(self.backend, self.action, coverage)
            self.assertTrue(lanes)
            self.assertTrue(all(inputs['incremental'] == 'false' for _, _, inputs, *_ in lanes))
        graph_lanes = {lane for lane, _, inputs, *_ in cache_lanes(self.warm, self.action)
                       if inputs.get('incremental') == 'true'}
        self.assertEqual(graph_lanes, {
            'rust-clippy', 'features-aws-kms-rust-features', 'features-gcp-kms-rust-features',
            'features-aws-and-gcp-kms-rust-features', 'cli-cli-test',
            'coverage-cli-coverage-cli', 'permissions-test',
        })
        self.assertEqual(self.backend['env']['CARGO_INCREMENTAL'], 0)
        for name in ('backend-test-archive', 'coverage-backend-archive'):
            self.assertEqual(self.warm['jobs'][name]['env']['CARGO_INCREMENTAL'], 0)
        inventory = next(s for s in self.action['runs']['steps'] if s.get('name') == 'Record restored graph sizes')
        self.assertEqual(inventory['if'], "inputs.incremental == 'true'")

    def test_reserve_input_controls_sdk_reclamation_and_disk_budget(self):
        self.assertEqual(self.action['inputs']['backend-reserve']['default'], 'false')
        prepare = next(s for s in self.action['runs']['steps']
                       if s.get('name') == 'Prepare disk before dependency restore')
        self.assertEqual(prepare['if'], "runner.os == 'Linux' && inputs.backend-reserve == 'true'")
        self.assertEqual(prepare['run'], 'python3 .github/scripts/backend_disk.py before-dependencies')
        guard = next(s for s in self.action['runs']['steps'] if s.get('id') == 'incremental-space')
        self.assertEqual(guard['env'], {'BACKEND_RESERVE': '${{ inputs.backend-reserve }}'})
        self.assertEqual(guard['run'],
                         'python3 .github/scripts/backend_disk.py before-incremental --backend-reserve "$BACKEND_RESERVE"')
        for path in Path('.github/workflows').glob('*.yml'):
            for job_id, job in load(str(path))['jobs'].items():
                steps = job.get('steps', [])
                reserve = any(s.get('run') == 'bash .github/scripts/prepare-backend-runner.sh'
                              for s in steps)
                for step in steps:
                    if step.get('uses') == CACHE_ACTION:
                        with self.subTest(workflow=path.name, job=job_id):
                            self.assertEqual(step['with'].get('backend-reserve', 'false'),
                                             'true' if reserve else 'false')
                            if not reserve:
                                for command in steps:
                                    self.assertNotRegex(command.get('run', ''), r'nextest(?:\s+archive|-archive)')

    def test_manual_dispatch_saves_branch_caches_and_push_saves_main_only(self):
        policy = next(step for step in self.action["runs"]["steps"] if step.get("id") == "save-policy")
        rust = next(step for step in self.action["runs"]["steps"]
                    if step.get("uses", "").startswith("Swatinem/rust-cache@"))
        self.assertEqual(rust["with"]["save-if"], "${{ steps.save-policy.outputs.enabled }}")
        self.assertEqual(self.action["outputs"]["save-enabled"]["value"],
                         "${{ steps.save-policy.outputs.enabled }}")
        cases = {
            ("false", "push", "refs/heads/main"): "false",
            ("true", "push", "refs/heads/main"): "true",
            ("true", "push", "refs/heads/feature"): "false",
            ("true", "workflow_dispatch", "refs/heads/feature"): "true",
            ("true", "workflow_dispatch", "refs/heads/main"): "true",
            ("false", "workflow_dispatch", "refs/heads/feature"): "false",
            ("true", "pull_request", "refs/pull/1/merge"): "false",
        }
        for (save_if, event, ref), expected in cases.items():
            with self.subTest(save_if=save_if, event=event, ref=ref):
                emitted = run_bash(policy["run"], {"SAVE_IF": save_if, "EVENT": event, "REF": ref})
                self.assertEqual(emitted["enabled"], expected)
        self.assertEqual(self.warm["on"]["push"]["branches"], ["main"])
        self.assertIn("workflow_dispatch", self.warm["on"])

    def test_reserved_disk_is_released_after_compilation_before_cache_save(self):
        prepare = "bash .github/scripts/prepare-backend-runner.sh"
        for job in self.warm["jobs"].values():
            steps = job["steps"]
            reservations = [i for i, s in enumerate(steps) if s.get("run") == prepare]
            releases = [i for i, s in enumerate(steps) if s.get("run") == prepare + " --release"]
            self.assertEqual(len(releases), len(reservations))
            if not reservations:
                continue
            release, = releases
            compiles = [i for i, s in enumerate(steps) if "cargo " in s.get("run", "")]
            self.assertLess(reservations[0], min(compiles))
            self.assertLess(max(compiles), release)
            saves = [i for i, s in enumerate(steps) if s.get("uses") == SAVE_ACTION]
            self.assertTrue(all(release < i for i in saves))

    def test_restore_precedes_swap_and_new_graphs_have_bounded_producers(self):
        for workflow in (self.warm, self.ci, self.backend):
            for job in workflow['jobs'].values():
                steps = job.get('steps', [])
                caches = [i for i, s in enumerate(steps) if s.get('uses') == CACHE_ACTION]
                reserves = [i for i, s in enumerate(steps)
                            if s.get('run') == 'bash .github/scripts/prepare-backend-runner.sh']
                if caches and reserves:
                    self.assertLess(caches[0], reserves[0])
        steps = self.action['runs']['steps']
        restore = next(s for s in steps if s.get('uses') == 'actions/cache/restore@v4')
        self.assertIn("steps.incremental-space.outputs.allowed == 'true'", restore['if'])
        guard = next(s for s in steps if s.get('id') == 'incremental-space')
        self.assertIn('backend_disk.py before-incremental', guard['run'])
        self.assertLess(steps.index(guard), steps.index(restore))
        self.assertIn('incremental_cache.py prune', self.save['runs']['steps'][0]['run'])
        self.assertIn('backend_disk.py before-save', self.save['runs']['steps'][0]['run'])

    def test_readers_compile_without_incremental_on_miss_or_skipped_restore(self):
        step = next(s for s in self.action['runs']['steps'] if s.get('name') == 'Select incremental compilation policy')
        self.assertEqual(step['env'], {
            'REQUESTED': '${{ inputs.incremental }}',
            'PRODUCER': '${{ steps.save-policy.outputs.enabled }}',
            'RESTORED_KEY': '${{ steps.incremental-restore.outputs.cache-matched-key }}',
            'RESTORE_ALLOWED': '${{ steps.incremental-space.outputs.allowed }}',
        })
        restore = next(s for s in self.action['runs']['steps'] if s.get('id') == 'incremental-restore')
        self.assertEqual(restore['uses'], 'actions/cache/restore@v4')
        for requested, producer, allowed, matched in itertools.product(
                ('true', 'false'), ('true', 'false'), ('true', 'false', ''), ('', 'fallback-key')):
            with self.subTest(requested=requested, producer=producer, allowed=allowed, matched=matched), \
                 tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                script = root / '.github/scripts/incremental_cache.py'
                script.parent.mkdir(parents=True)
                script.write_text('from pathlib import Path\nPath("marked").touch()\n')
                output = root / 'environment'
                result = subprocess.run(['bash', '-c', step['run']], cwd=root, env={**os.environ,
                    'REQUESTED': requested, 'PRODUCER': producer, 'RESTORED_KEY': matched,
                    'RESTORE_ALLOWED': allowed,
                    'GITHUB_ENV': str(output)}, text=True, capture_output=True)
                reason = ('not-requested' if requested != 'true' else 'disk-guard' if allowed != 'true'
                          else 'hit' if matched else 'miss')
                failed = producer == 'true' and reason == 'disk-guard'
                self.assertEqual(result.returncode, int(failed), result.stderr)
                self.assertEqual('::error::' in result.stdout, failed)
                enabled = requested == 'true' and allowed == 'true' and (producer == 'true' or bool(matched))
                self.assertEqual(output.read_text(), f'CARGO_INCREMENTAL={int(enabled)}\n')
                self.assertIn(f'[ci-incremental] enabled={int(enabled)} producer={producer} restored={matched} reason={reason}',
                              result.stdout)
                self.assertEqual((root / 'marked').exists(), enabled)
        dependency = next(s for s in self.action['runs']['steps'] if s.get('uses') == 'Swatinem/rust-cache@v2')
        self.assertEqual(dependency['with']['prefix-key'],
                         "${{ inputs.incremental == 'true' && 'v1-rust-incremental' || 'v0-rust' }}")

    def test_backend_archive_preserves_compiler_cache_but_clears_counters(self):
        for job in (self.backend["jobs"]["build"], self.warm["jobs"]["coverage-backend-archive"]):
            steps = job["steps"]
            clean, = [i for i, s in enumerate(steps)
                      if s.get("run") == "cargo llvm-cov clean --profraw-only"]
            build, = [i for i, s in enumerate(steps)
                      if s.get("name") == "Build instrumented test archive"]
            self.assertLess(clean, build)
            self.assertIn("--no-clean", steps[build]["run"])
        steps = self.backend["jobs"]["build"]["steps"]
        check = next(s for s in steps if s.get("name") == "Verify cached coverage archive isolation")
        self.assertEqual(check["if"], "inputs.coverage")
        self.assertEqual(check["run"], "python3 .github/scripts/test_coverage_archive_cache.py")
        for s in self.backend["jobs"]["report"]["steps"]:
            if "cargo llvm-cov report" in s.get("run", ""):
                for line in s["run"].splitlines():
                    if "cargo llvm-cov report" in line:
                        self.assertIn("--nextest-archive-file backend-tests.tar.zst", line)

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
        self.assertEqual(self.warm["concurrency"], {"group": "ci-cache-warm-${{ github.ref }}", "cancel-in-progress": True})
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
        self.assertNotIn("cargo check", step["run"])
        self.assertEqual(workflow["env"]["RUSTFLAGS"], "-Dwarnings")
        clippy = next(step for step in workflow["jobs"]["rust-clippy"]["steps"]
                      if step.get("name") == "Compile clippy" or "cargo clippy --workspace" in step.get("run", ""))
        self.assertIn("cargo clippy --workspace --all-targets -- -D warnings", clippy["run"])

    def test_image_input_reader_and_warmer_check_the_same_production_closure(self):
        commands = []
        for workflow in (self.ci, self.warm):
            job = workflow["jobs"]["backend-image-inputs"]
            build = next(s for s in job["steps"] if s.get("name") == "Check backend release inputs")
            commands.append(build["run"])
            self.assertIn("cargo check --locked --release -p nyxid --bin nyxid-server --features gcp-kms", build["run"])
            self.assertIn("--message-format=json-render-diagnostics", build["run"])
            guard = next(s for s in job["steps"] if s.get("name") == "Check backend Docker embedded assets")
            self.assertEqual(guard["run"], 'python3 scripts/check-backend-docker-embeds.py --cargo-messages "$RUNNER_TEMP/backend-image-inputs.jsonl"')
        self.assertEqual(commands[0], commands[1])
        self.assertEqual(self.ci["jobs"]["backend-image-inputs"]["name"], "Backend Image Inputs")
        image = self.ci["jobs"]["backend-image-smoke"]
        self.assertEqual(image["name"], "Backend Release Image")
        self.assertTrue(any("--file backend/Dockerfile" in s.get("run", "") for s in image["steps"]))

    def test_warm_compiles_record_resources_without_git_stamp(self):
        self.assertNotIn("with-git-stamp.py", self.warm_text)
        compile = re.compile(r"\bcargo\s+(?:build|check|test|clippy|nextest|llvm-cov)\b")
        for job_id, job in self.warm["jobs"].items():
            saw_compile = False
            for step in job["steps"]:
                run = step.get("run", "")
                if "cargo llvm-cov clean" in run:
                    self.assertNotIn("ci_resources.py", run)
                    continue
                if compile.search(run):
                    saw_compile = True
                    self.assertIn("python3 .github/scripts/ci_resources.py ", run, job_id)
            self.assertTrue(saw_compile, job_id)
            upload = next(step for step in job["steps"]
                          if step.get("uses") == "actions/upload-artifact@v4"
                          and step.get("with", {}).get("path") == "${{ runner.temp }}/nyxid-ci-resources")
            self.assertEqual(upload["if"], "always()")
            self.assertTrue(upload.get("continue-on-error"))
            self.assertEqual(upload["with"]["retention-days"], 14)
            self.assertEqual(upload["with"]["if-no-files-found"], "ignore")
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

    def test_cli_coverage_drops_stale_objects_before_no_report_measurement(self):
        steps = self.ci["jobs"]["coverage-cli"]["steps"]
        cache = next(i for i, step in enumerate(steps)
                     if step.get("uses") == CACHE_ACTION)
        clean = next(i for i, step in enumerate(steps)
                     if step.get("run") == "cargo llvm-cov clean --workspace")
        build = next(i for i, step in enumerate(steps)
                     if "cargo llvm-cov --package nyxid-cli --no-report" in step.get("run", ""))
        report = next(i for i, step in enumerate(steps)
                      if "cargo llvm-cov report --package nyxid-cli --lcov" in step.get("run", ""))
        self.assertLess(cache, clean)
        self.assertLess(clean, build)
        self.assertLess(build, report)
        self.assertNotIn("if", steps[clean])
        self.assertFalse(steps[clean].get("continue-on-error", False))


if __name__ == "__main__":
    unittest.main()
