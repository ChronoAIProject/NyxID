"""Prove cached nextest archives report only the current instrumented objects."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


TOOLS = ("cargo", "cargo-nextest", "cargo-llvm-cov")
HAVE_TOOLS = all(shutil.which(tool) for tool in TOOLS)


@unittest.skipUnless(HAVE_TOOLS, "requires the pinned nextest and llvm-cov tools")
class CoverageArchiveCacheTests(unittest.TestCase):
    def test_removed_sources_and_renamed_binary_match_a_clean_report(self):
        with tempfile.TemporaryDirectory(prefix="coverage archive ") as directory:
            root = Path(directory)
            (root / "src").mkdir()
            target = root / "target"
            env = {**os.environ, "CARGO_TARGET_DIR": str(target), "CARGO_INCREMENTAL": "1", "CARGO_TERM_COLOR": "never",
                   "RUSTFLAGS": "-Dwarnings", "LLVM_PROFILE_FILE_NAME": "fixture-%4m.profraw"}
            for name in ("CARGO_ENCODED_RUSTFLAGS", "LLVM_PROFILE_FILE", "CARGO_LLVM_COV_TARGET_DIR",
                         "CARGO_LLVM_COV_BUILD_DIR", "CARGO_BUILD_TARGET_DIR"):
                env.pop(name, None)

            def cargo(*args):
                result = subprocess.run(["cargo", *args], cwd=root, env=env,
                                        capture_output=True, text=True, timeout=120)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                return result

            def sources(binary, module):
                (root / "Cargo.toml").write_text(
                    '[package]\nname="coverage-fixture"\nversion="0.1.0"\nedition="2021"\n'
                    '[workspace]\n[[bin]]\n'
                    f'name="{binary}"\npath="src/main.rs"\n'
                    '[lints.rust]\nunexpected_cfgs={level="warn", check-cfg=["cfg(coverage)"]}\n'
                )
                (root / "src/main.rs").write_text(
                    f'mod {module};\nfn main() {{ assert_eq!({module}::value(), 7); }}\n'
                    f'#[test]\nfn {module}_contract() {{ main(); }}\n'
                    '#[test]\n#[ignore]\nfn existing_ignored() { main(); }\n'
                    '#[cfg(coverage)]\n#[test]\nfn coverage_contract() { main(); }\n'
                )
                (root / f"src/{module}.rs").write_text('pub fn value() -> u8 { 7 }\n')

            def archive(name, cached=False):
                path = root / f"{name}.tar.zst"
                extra = ["--no-clean"] if cached else []
                cargo("llvm-cov", "nextest-archive", "--package", "coverage-fixture",
                      "--archive-file", str(path), *extra)
                return path

            def measure(path):
                cargo("llvm-cov", "clean", "--profraw-only")
                # Each execution job starts without a prior extracted archive.
                shutil.rmtree(target / "llvm-cov-target/target", ignore_errors=True)
                cargo("llvm-cov", "nextest", "--archive-file", str(path), "--no-report")
                inventory = json.loads(cargo("nextest", "list", "--archive-file", str(path),
                                             "--message-format", "json").stdout)
                report = json.loads(cargo("llvm-cov", "report", "--nextest-archive-file", str(path),
                                          "--package", "coverage-fixture", "--json").stdout)
                identities = {binary: suite["testcases"]
                              for binary, suite in inventory["rust-suites"].items()}
                return identities, report

            sources("old-binary", "removed")
            old = archive("old")
            measure(old)
            old_objects = list((target / "llvm-cov-target/debug/deps").glob("old_binary-*"))
            self.assertTrue(old_objects)
            sources("current-binary", "current")
            (root / "src/removed.rs").unlink()
            cargo("llvm-cov", "clean", "--profraw-only")
            cached = archive("cached", cached=True)
            unchanged = cargo("llvm-cov", "nextest-archive", "--package", "coverage-fixture",
                              "--archive-file", str(cached), "--no-clean")
            self.assertNotIn("Compiling coverage-fixture", unchanged.stderr)
            # The old objects really remain, so equality below tests selection,
            # rather than accidentally relying on a clean target directory.
            self.assertTrue(all(path.exists() for path in old_objects))
            cached_inventory, cached_report = measure(cached)
            self.assertEqual(set(cached_inventory), {"coverage-fixture::bin/current-binary"})
            self.assertEqual(set(next(iter(cached_inventory.values()))),
                             {"current_contract", "coverage_contract", "existing_ignored"})
            files = {Path(row["filename"]).name for data in cached_report["data"]
                     for row in data["files"]}
            self.assertIn("current.rs", files)
            self.assertNotIn("removed.rs", files)
            clean_inventory, clean_report = measure(archive("clean"))
            self.assertEqual(cached_inventory, clean_inventory)
            self.assertEqual(cached_report, clean_report)


if __name__ == "__main__":
    if not HAVE_TOOLS:
        raise SystemExit("Install the workflow's pinned cargo-nextest and cargo-llvm-cov first")
    unittest.main()
