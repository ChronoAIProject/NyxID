import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from ci_gate_reuse import strict_load_workflow


SCRIPT = Path(__file__).with_name("dist_cargo.py")
SPEC = importlib.util.spec_from_file_location("dist_cargo", SCRIPT)
dist_cargo = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(dist_cargo)


class DistCargoTests(unittest.TestCase):
    def test_pr_release_matrix_builds_even_on_exact_cache_hit(self):
        workflow = strict_load_workflow(Path('.github/workflows/release-cache.yml').read_text())
        steps = workflow['jobs']['warm']['steps']
        for name in ('Install dist', 'Install dependencies', 'Build artifacts'):
            step = next(s for s in steps if s.get('name') == name)
            self.assertEqual(step['if'],
                             "github.event_name == 'pull_request' || steps.cache.outputs.cache-hit != 'true'")
        cache = next(s for s in steps if s.get('id') == 'cache')
        self.assertEqual(cache['with']['save-if'], "${{ github.event_name != 'pull_request' }}")

    def test_workspace_features_and_dist_options_are_retained(self):
        args = ["build", "--profile", "dist", "--message-format=json-render-diagnostics",
                "--target", "aarch64-apple-darwin", "--workspace"]
        self.assertEqual(dist_cargo.build_args(args), [*args, "--bin", "nyxid", "--locked"])

    def test_metadata_and_version_queries_pass_through(self):
        for args in (["--version", "--verbose"], ["metadata", "--format-version", "1"]):
            self.assertEqual(dist_cargo.build_args(args), args)

    def test_changed_build_contract_fails(self):
        for args in (
            ["build", "--profile", "dist", "--package", "nyxid-cli"],
            ["build", "--workspace", "--release"],
            ["build", "--workspace", "--profile"],
            ["build", "--workspace", "--profile", "release"],
            ["build", "--workspace", "--profile", "dist", "--bin=other"],
            ["build", "--workspace", "--profile", "dist", "--all-targets"],
        ):
            with self.subTest(args=args), self.assertRaises(ValueError):
                dist_cargo.build_args(args)

    def test_other_package_and_target_selectors_fail(self):
        for selection in (
            "-pnyxid-cli", "--exclude=nyxid", "--manifest-path=cli/Cargo.toml",
            "--test=contract", "--tests", "--examples", "--example=demo",
            "--bench=perf", "--benches", "--all",
        ):
            with self.subTest(selection=selection), self.assertRaises(ValueError):
                dist_cargo.build_args(["build", "--workspace", "--profile", "dist", selection])

    def test_exec_preserves_warnings_and_restores_cargo_for_build_scripts(self):
        with tempfile.TemporaryDirectory() as directory:
            cargo = Path(directory) / "cargo"
            cargo.write_text(
                "#!/usr/bin/env python3\n"
                "import json, os, sys\n"
                "print(json.dumps([sys.argv[1:], os.environ['CARGO'], os.environ['RUSTFLAGS']]))\n"
            )
            cargo.chmod(0o755)
            result = subprocess.run(
                [sys.executable, str(SCRIPT), "build", "--workspace", "--profile", "dist"],
                env={**os.environ, "NYXID_DIST_CARGO": str(cargo), "CARGO": str(SCRIPT),
                     "RUSTFLAGS": "-Dwarnings"},
                check=True, capture_output=True, text=True,
            )
            args, nested_cargo, flags = json.loads(result.stdout)
            self.assertEqual(args, ["build", "--workspace", "--profile", "dist", "--bin", "nyxid", "--locked"])
            self.assertEqual(nested_cargo, str(cargo))
            self.assertEqual(flags, "-Dwarnings")


if __name__ == "__main__":
    unittest.main()
