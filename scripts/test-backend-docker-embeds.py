#!/usr/bin/env python3
"""Run the embed guard against a real no-Git Cargo build and generated identity."""
import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import unittest

ROOT = Path(__file__).resolve().parents[1]


def module(name, filename):
    spec = importlib.util.spec_from_file_location(name, ROOT / "scripts" / filename)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


fixtures = module("embed_export_fixtures", "test-export-build-source.py")
guard = module("backend_embed_guard", "check-backend-docker-embeds.py")


class BackendEmbeds(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.fixture = fixtures.SourceExport("test_exact_commit_export_is_no_git_and_repeatable")
        cls.fixture.setUp()
        root = cls.fixture.root
        (root / "Cargo.toml").write_text('[workspace]\nmembers=["backend"]\nresolver="3"\n')
        (root / "build-support").mkdir()
        (root / "build-support/source_identity.rs").write_bytes((ROOT / "build-support/source_identity.rs").read_bytes())
        (root / "backend/src").mkdir(parents=True)
        (root / "backend/assets").mkdir()
        (root / "backend/assets/content.txt").write_text("actual include_str input")
        (root / "backend/Cargo.toml").write_text('[package]\nname="nyxid"\nversion="0.1.0"\nedition="2024"\n')
        (root / "backend/build.rs").write_text(
            '#[path="../build-support/source_identity.rs"] mod identity;\nfn main(){identity::emit();}\n')
        (root / "backend/src/main.rs").write_text(
            'include!(concat!(env!("OUT_DIR"),"/nyxid_build_identity.rs"));\n'
            'fn main(){retain();println!("{}",include_str!("../assets/content.txt"));}\n')
        (root / "backend/Dockerfile").write_text(
            "FROM scratch AS builder\nCOPY backend/ backend/\nCOPY build-support/ build-support/\nFROM scratch\n")
        (root / ".dockerignore").write_text("target/\n.git\n")
        subprocess.run(["cargo", "generate-lockfile", "--offline"], cwd=root, check=True, capture_output=True)
        cls.fixture.commit()
        cls.destination, exported = cls.fixture.exported()
        cls.root = (cls.destination / "source").resolve()
        cls.messages = cls.destination / "build.jsonl"
        env = {key: value for key, value in os.environ.items()
               if key not in ("CARGO_TARGET_DIR", "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS")}
        env.update(NYXID_GIT_HASH=exported["source"]["commit"],
                   SOURCE_DATE_EPOCH=str(exported["source"]["epoch"]),
                   CARGO_INCREMENTAL="0", CARGO_BUILD_JOBS="1")
        with cls.messages.open("w") as output:
            result = subprocess.run(["cargo", "build", "--locked", "--offline",
                                     "--message-format=json-render-diagnostics"], cwd=cls.root,
                                    env=env, stdout=output, stderr=subprocess.PIPE, text=True)
        if result.returncode:
            raise RuntimeError(result.stderr)
        cls.original_root = guard.REPO_ROOT
        guard.REPO_ROOT = cls.root
        cls.dep_info = cls.root / "target/debug/nyxid.d"
        cls.dockerfile = cls.root / "backend/Dockerfile"
        cls.original_dockerfile = cls.dockerfile.read_text()

    @classmethod
    def tearDownClass(cls):
        guard.REPO_ROOT = cls.original_root
        cls.fixture.doCleanups()

    def setUp(self):
        self.dockerfile.write_text(self.original_dockerfile)

    def check(self, dep_info=None, messages=None):
        with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            return guard.check(dep_info or self.dep_info, self.dockerfile,
                               self.root / ".dockerignore", messages or self.messages)

    def test_actual_exported_build_classifies_only_recorded_identity_output(self):
        self.assertEqual(self.check(), 0)
        outputs = guard.generated_identity_outputs(self.messages)
        self.assertEqual(len(outputs), 1)
        self.assertIn(next(iter(outputs)), guard.parse_dep_info(self.dep_info))
        self.assertNotIn(self.root / "source.txt", guard.parse_dep_info(self.dep_info))
        with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(guard.check(self.dep_info, self.dockerfile, self.root / ".dockerignore"), 1)

    def test_missing_actual_include_str_copy_still_fails(self):
        self.dockerfile.write_text(self.original_dockerfile.replace(
            "COPY backend/ backend/", "COPY backend/src backend/src\nCOPY backend/build.rs backend/build.rs"))
        self.assertEqual(self.check(), 1)

    def test_arbitrary_target_source_and_git_watch_are_not_exempt(self):
        for name in ("target/arbitrary.rs", ".git/HEAD"):
            with self.subTest(path=name):
                path = self.root / name
                path.parent.mkdir(exist_ok=True)
                path.write_text("unmapped input")
                dep_info = self.destination / "extra.d"
                dep_info.write_text(self.dep_info.read_text().rstrip() + " " + str(path) + "\n")
                self.assertEqual(self.check(dep_info), 1)
                path.unlink()
                if name.startswith(".git/"):
                    path.parent.rmdir()

    def test_failed_missing_or_forged_completion_cannot_qualify_generated_output(self):
        lines = [json.loads(line) for line in self.messages.read_text().splitlines()]
        for mode in ("failed", "missing", "duplicate"):
            with self.subTest(mode=mode):
                altered = [line for line in lines if line.get("reason") != "build-finished"]
                if mode == "failed":
                    altered.append({"reason": "build-finished", "success": False})
                if mode == "duplicate":
                    altered.extend([{"reason": "build-finished", "success": True}] * 2)
                path = self.destination / (mode + ".jsonl")
                path.write_text("".join(json.dumps(line) + "\n" for line in altered))
                with self.assertRaisesRegex(ValueError, "successful completed"):
                    guard.generated_identity_outputs(path)


if __name__ == "__main__":
    unittest.main()
