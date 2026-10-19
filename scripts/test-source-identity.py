#!/usr/bin/env python3
"""Execute build-script and stripped-binary/cache regressions without app builds."""
import ast
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
START = b"NYXID_BUILD_IDENTITY_V1\0"
END = b"\0NYXID_BUILD_IDENTITY_END\0"
COMMIT = "a" * 40


def marker(data):
    if data.count(START) != 1 or data.count(END) != 1:
        raise ValueError("exactly one framed identity required")
    return json.loads(data.split(START)[1].split(END)[0])


def environment():
    env = dict(os.environ)
    # Inspection must not refresh Git's index mtime and invalidate the next
    # unchanged-build control. Essential add/commit/worktree writes still run.
    env["GIT_OPTIONAL_LOCKS"] = "0"
    for key in ("NYXID_GIT_HASH", "SOURCE_DATE_EPOCH", "TARGET", "CARGO_TARGET_DIR", "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS"):
        env.pop(key, None)
    return env


class SourceIdentity(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.directory = tempfile.TemporaryDirectory(prefix="nyxid-identity-tests-")
        cls.root = Path(cls.directory.name)
        wrapper = cls.root / "build.rs"
        wrapper.write_text('#[path = ' + json.dumps(str(ROOT / "build-support/source_identity.rs")) + '] mod identity;\nfn main() { identity::emit(); }\n')
        cls.wrapper = wrapper
        cls.executable = cls.root / "build-script"
        subprocess.run(["rustc", "--edition=2024", str(wrapper), "-o", str(cls.executable)], check=True, capture_output=True)

    @classmethod
    def tearDownClass(cls):
        cls.directory.cleanup()

    def emit(self, values, success=True):
        with tempfile.TemporaryDirectory(dir=self.root) as directory:
            directory = Path(directory)
            env = environment()
            env.update({"OUT_DIR": str(directory), "CARGO_MANIFEST_DIR": str(directory), "CARGO_PKG_NAME": "fixture", "CARGO_PKG_VERSION": "0.71.0", "TARGET": "aarch64-unknown-linux-gnu"})
            env.update(values)
            result = subprocess.run([str(self.executable)], env=env, capture_output=True)
            self.assertEqual(result.returncode == 0, success, result.stderr.decode())
            if not success:
                self.assertFalse((directory / "nyxid_build_identity.rs").exists())
                return None
            generated = (directory / "nyxid_build_identity.rs").read_text()
            data = bytes(ast.literal_eval(re.search(r"= (\[[^\n]+\]);", generated)[1]))
            return marker(data)

    def test_exported_identity_requires_full_commit_and_epoch(self):
        value = self.emit({"NYXID_GIT_HASH": COMMIT, "SOURCE_DATE_EPOCH": "1792365742"})
        self.assertEqual(value, {"schema": "nyxid.build-identity.v1", "package": "fixture", "package_version": "0.71.0", "target": "aarch64-unknown-linux-gnu", "commit": COMMIT, "source_date_epoch": 1792365742, "source_state": "exported"})
        self.emit({"NYXID_GIT_HASH": COMMIT}, False)
        self.emit({"SOURCE_DATE_EPOCH": "1792365742"}, False)

    def test_invalid_explicit_inputs_fail_before_marker_creation(self):
        for value in ("", "unknown", "a" * 12, "A" * 40, COMMIT + "\n"):
            with self.subTest(commit=value):
                self.emit({"NYXID_GIT_HASH": value, "SOURCE_DATE_EPOCH": "1"}, False)
        for value in ("", "-1", "01", "1.0", "253402300800", "1\n"):
            with self.subTest(epoch=value):
                self.emit({"NYXID_GIT_HASH": COMMIT, "SOURCE_DATE_EPOCH": value}, False)

    def test_no_git_local_build_reports_unknown(self):
        value = self.emit({})
        self.assertEqual(value["source_state"], "unknown")
        self.assertEqual(value["commit"], "unknown")
        self.assertIsNone(value["source_date_epoch"])

    def fixture(self, root, relative_helper=False):
        root.mkdir()
        (root / "app/src").mkdir(parents=True)
        (root / "Cargo.toml").write_text('[workspace]\nmembers=["app"]\nresolver="3"\n[profile.release]\nstrip=true\n')
        (root / "app/Cargo.toml").write_text('[package]\nname="identity-fixture"\nversion="0.1.0"\nedition="2024"\n')
        if relative_helper:
            (root / "support").mkdir()
            (root / "support/source_identity.rs").write_bytes((ROOT / "build-support/source_identity.rs").read_bytes())
            (root / "app/build.rs").write_text('#[path = "../support/source_identity.rs"] mod identity;\nfn main() { identity::emit(); }\n')
        else:
            (root / "app/build.rs").write_bytes(self.wrapper.read_bytes())
        (root / "app/src/main.rs").write_text('include!(concat!(env!("OUT_DIR"), "/nyxid_build_identity.rs"));\nfn main() { retain(); println!("before"); }\n')
        self.command(root, ["cargo", "generate-lockfile", "--offline"])
        self.command(root, ["git", "init", "-b", "main"])
        self.command(root, ["git", "config", "user.email", "fixture@local.invalid"])
        self.command(root, ["git", "config", "user.name", "Identity fixture"])
        self.command(root, ["git", "add", "."])
        self.command(root, ["git", "commit", "-m", "fixture source"])

    def command(self, root, arguments, extra=None, success=True):
        env = environment()
        env.update({"CARGO_BUILD_JOBS": "1", "CARGO_INCREMENTAL": "0"})
        if extra:
            env.update(extra)
        result = subprocess.run(arguments, cwd=root, env=env, capture_output=True, text=True)
        self.assertEqual(result.returncode == 0, success, result.stderr)
        self.last_stderr = result.stderr
        return result.stdout.strip()

    def build(self, root, target, extra=None, success=True):
        values = {"CARGO_TARGET_DIR": str(target)}
        values.update(extra or {})
        self.command(root / "app", ["cargo", "build", "--locked", "--offline", "--release", "--verbose"], values, success)
        if not success:
            return None
        binary = target / "release/identity-fixture"
        value = marker(binary.read_bytes())
        if os.environ.get("NYXID_ARTIFACT_INFO"):
            build_stderr = self.last_stderr
            parsed = json.loads(self.command(root, [os.environ["NYXID_ARTIFACT_INFO"], "--identity-only", str(binary)]))
            self.last_stderr = build_stderr
            self.assertEqual(parsed["build_identity"], value)
            self.assertEqual(parsed["subject"]["size"], binary.stat().st_size)
        self.assertNotIn(str(root), json.dumps(value))
        self.assertNotIn(str(ROOT), json.dumps(value))
        return value

    def assert_fresh(self, root, target, expected):
        self.assertEqual(self.build(root, target), expected)
        self.assertIn("Fresh identity-fixture", self.last_stderr)
        self.assertNotIn("build-script-build`", self.last_stderr)

    def test_sibling_dependency_edits_invalidate_identity_and_unchanged_builds_are_fresh(self):
        for relative_helper in (False, True):
            with self.subTest(relative_helper=relative_helper), tempfile.TemporaryDirectory(dir=self.root) as temp:
                directory = Path(temp)
                root, target = directory / "repo", directory / "target"
                self.fixture(root, relative_helper)
                (root / "dep/src").mkdir(parents=True)
                (root / "dep/Cargo.toml").write_text('[package]\nname="fixture-dependency"\nversion="0.1.0"\nedition="2024"\n')
                dependency = root / "dep/src/lib.rs"
                dependency.write_text('pub fn value() -> u8 { 2 }\n')
                with (root / "app/Cargo.toml").open("a") as output:
                    output.write('[dependencies]\nfixture-dependency={path="../dep"}\n')
                source = root / "app/src/main.rs"
                source.write_text(source.read_text().replace('println!("before")', 'println!("{}", fixture_dependency::value())'))
                self.command(root, ["cargo", "generate-lockfile", "--offline"])
                self.command(root, ["git", "add", "."])
                self.command(root, ["git", "commit", "-m", "path dependency"])
                first = self.build(root, target)
                self.assertEqual(first["source_state"], "git-unverified")
                self.assert_fresh(root, target, first)
                dependency.write_text('pub fn value() -> u8 { 3 }\n')
                dirty = self.build(root, target)
                self.assertEqual(dirty["source_state"], "git-dirty")
                self.assertEqual(dirty["commit"], first["commit"])
                self.assertEqual(self.command(root, [str(target / "release/identity-fixture")]), "3")
                self.assert_fresh(root, target, dirty)

    def test_new_untracked_build_input_creation_and_removal_cannot_certify_clean_source(self):
        with tempfile.TemporaryDirectory(dir=self.root) as temp:
            directory = Path(temp)
            root, target = directory / "repo", directory / "target"
            self.fixture(root, True)
            (root / "dep/src").mkdir(parents=True)
            (root / "dep/data").mkdir()
            (root / "dep/data/.keep").write_text("")
            (root / ".gitignore").write_text("node_modules/\n")
            (root / "dep/Cargo.toml").write_text('[package]\nname="fixture-dependency"\nversion="0.1.0"\nedition="2024"\n')
            (root / "dep/src/lib.rs").write_text('include!(concat!(env!("OUT_DIR"), "/value.rs"));\n')
            (root / "dep/build.rs").write_text(
                'fn main() { println!("cargo:rerun-if-changed=data"); '
                'let value=std::fs::read_to_string("data/new.txt").unwrap_or("2".into()); '
                'let out=std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("value.rs"); '
                'std::fs::write(out, format!("pub fn value() -> u8 {{ {} }}", value.trim())).unwrap(); }\n')
            with (root / "app/Cargo.toml").open("a") as output:
                output.write('[dependencies]\nfixture-dependency={path="../dep"}\n')
            source = root / "app/src/main.rs"
            source.write_text(source.read_text().replace('println!("before")', 'println!("{}", fixture_dependency::value())'))
            self.command(root, ["cargo", "generate-lockfile", "--offline"])
            self.command(root, ["git", "add", "."])
            self.command(root, ["git", "commit", "-m", "generated dependency input"])
            binary = target / "release/identity-fixture"
            first = self.build(root, target)
            self.assertEqual(first["source_state"], "git-unverified")
            self.assertEqual(self.command(root, [str(binary)]), "2")
            first_bytes = binary.read_bytes()
            self.assert_fresh(root, target, first)
            (root / "node_modules").mkdir()
            (root / "node_modules/generated").write_text("ignored output")
            self.assert_fresh(root, target, first)
            new_input = root / "dep/data/new.txt"
            new_input.write_text("3")
            created = self.build(root, target)
            self.assertIn(created["source_state"], ("git-unverified", "git-dirty"))
            self.assertEqual(self.command(root, [str(binary)]), "3")
            self.assertNotEqual(first_bytes, binary.read_bytes())
            self.assertEqual(self.command(root, ["git", "status", "--porcelain"]), "?? dep/data/new.txt")
            self.assert_fresh(root, target, created)
            new_input.unlink()
            removed = self.build(root, target)
            self.assertIn(removed["source_state"], ("git-unverified", "git-dirty"))
            self.assertEqual(self.command(root, [str(binary)]), "2")
            self.assert_fresh(root, target, removed)

    def test_stripped_binary_tracks_unstaged_source_and_commit_in_same_target_directory(self):
        with tempfile.TemporaryDirectory(dir=self.root) as temp:
            directory = Path(temp)
            root, target = directory / "repo", directory / "target"
            self.fixture(root)
            first = self.build(root, target)
            self.assertEqual(first["source_state"], "git-unverified")
            self.assert_fresh(root, target, first)
            source = root / "app/src/main.rs"
            source.write_text(source.read_text().replace("before", "after"))
            dirty = self.build(root, target)
            self.assertEqual(dirty["source_state"], "git-dirty")
            self.assertEqual(dirty["commit"], first["commit"])
            self.assertEqual(self.command(root, [str(target / "release/identity-fixture")]), "after")
            self.command(root, ["git", "add", "."])
            self.command(root, ["git", "commit", "-m", "new source"])
            clean = self.build(root, target)
            self.assertEqual(clean["source_state"], "git-unverified")
            self.assertNotEqual(clean["commit"], first["commit"])
            self.build(root, target, {"NYXID_GIT_HASH": COMMIT, "SOURCE_DATE_EPOCH": "1"}, False)

    def test_linked_worktree_and_packed_ref_changes_invalidate_cache(self):
        with tempfile.TemporaryDirectory(dir=self.root) as temp:
            directory = Path(temp)
            root, linked, target = directory / "repo", directory / "linked", directory / "target"
            self.fixture(root)
            self.command(root, ["git", "worktree", "add", "-b", "linked", str(linked)])
            self.assertTrue((linked / ".git").is_file())
            first = self.build(linked, target)
            self.assertEqual(first["source_state"], "git-unverified")
            self.command(root, ["git", "pack-refs", "--all", "--prune"])
            self.command(linked, ["git", "commit", "--allow-empty", "-m", "linked change"])
            second = self.build(linked, target)
            self.assertNotEqual(second["commit"], first["commit"])
            self.assertEqual(second["commit"], self.command(linked, ["git", "rev-parse", "HEAD"]))
            self.assertEqual(second["source_state"], "git-unverified")


if __name__ == "__main__":
    unittest.main()
