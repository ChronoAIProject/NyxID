#!/usr/bin/env python3
"""Regression checks for the content-addressed Docker dependency recipe."""

import importlib.util
import re
import tempfile
import tomllib
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location(
    "recipe", ROOT / "scripts/docker-dependency-recipe.py"
)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def tree(root):
    return {str(path.relative_to(root)): path.read_bytes() for path in root.rglob("*") if path.is_file()}


class RecipeTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.source = Path(self.temp.name) / "source"
        self.output = Path(self.temp.name) / "output"
        self.source.mkdir()
        self.members = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["members"]
        for name in ["Cargo.toml", "Cargo.lock", *[f"{m}/Cargo.toml" for m in self.members]]:
            target = self.source / name
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes((ROOT / name).read_bytes())

    def test_release_version_bump_produces_identical_recipe(self):
        MODULE.recipe(self.source, self.output)
        before = tree(self.output)
        MODULE.VERSION = "9.8.7"
        self.addCleanup(setattr, MODULE, "VERSION", "0.0.0")
        bumped = Path(self.temp.name) / "bumped"
        MODULE.recipe(self.source, bumped)
        MODULE.VERSION = "0.0.0"
        MODULE.recipe(bumped, self.output)
        self.assertEqual(tree(self.output), before)

    def test_third_party_lock_blocks_are_byte_exact(self):
        MODULE.recipe(self.source, self.output)
        def external(path):
            return [block for block in re.split(r"(?m)(?=^\[\[package\]\])", path.read_text())
                    if block.startswith("[[package]]") and "source" in tomllib.loads(block)["package"][0]]
        self.assertEqual(external(self.source / "Cargo.lock"), external(self.output / "Cargo.lock"))

    def test_real_dependency_change_invalidates_recipe(self):
        MODULE.recipe(self.source, self.output)
        before = tree(self.output)
        path = self.source / "Cargo.toml"
        path.write_text(path.read_text().replace('mongodb = "3.5.2"', 'mongodb = "=3.5.2"'))
        MODULE.recipe(self.source, self.output)
        self.assertNotEqual(tree(self.output), before)

    def test_registry_package_with_member_name_keeps_its_version_and_checksum(self):
        block = '\n[[package]]\nname = "nyxid"\nversion = "0.75.0"\nsource = "registry+https://github.com/rust-lang/crates.io-index"\nchecksum = "abc"\n'
        lock = self.source / "Cargo.lock"
        lock.write_text(lock.read_text() + block)
        MODULE.recipe(self.source, self.output)
        self.assertTrue((self.output / "Cargo.lock").read_text().endswith(block))

    def test_version_qualified_local_edge_is_normalized(self):
        lock = self.source / "Cargo.lock"
        version = tomllib.loads((self.source / "cloud-auth/Cargo.toml").read_text())["package"]["version"]
        lock.write_text(lock.read_text().replace(' "nyxid-cloud-auth",', f' "nyxid-cloud-auth {version}",'))
        MODULE.recipe(self.source, self.output)
        self.assertIn(' "nyxid-cloud-auth 0.0.0",', (self.output / "Cargo.lock").read_text())

    def test_mismatched_member_lock_fails(self):
        path = self.source / "backend/Cargo.toml"
        path.write_text(MODULE.normalize_package(path.read_text()))
        with self.assertRaisesRegex(ValueError, "does not match"):
            MODULE.recipe(self.source, self.output)

    def test_versioned_path_requirement_fails_instead_of_weakening_it(self):
        path = self.source / "backend/Cargo.toml"
        path.write_text(path.read_text().replace('path = "../cloud-auth"', 'path = "../cloud-auth", version = "0.1"'))
        with self.assertRaisesRegex(ValueError, "versioned path"):
            MODULE.recipe(self.source, self.output)

    def test_source_files_stay_unchanged_and_versions_are_normalized(self):
        before = tree(self.source)
        MODULE.recipe(self.source, self.output)
        self.assertEqual(tree(self.source), before)
        for member in self.members:
            self.assertEqual(tomllib.loads((self.output / member / "Cargo.toml").read_text())["package"]["version"], "0.0.0")
        self.assertEqual(len(tree(self.output)), len(self.members) + 2)


if __name__ == "__main__":
    unittest.main()
