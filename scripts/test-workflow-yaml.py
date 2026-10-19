#!/usr/bin/env python3
import unittest
import yaml
from workflow_yaml import RUST_ACTION, load_unique, validate_rust_steps


class WorkflowYaml(unittest.TestCase):
    def test_conditional_step_duplicate_with_rejected(self):
        text = """jobs:
  coverage:
    steps:
      - uses: dtolnay/rust-toolchain@18f0487477426dce244608110aa2660c2893deb7
        with:
          toolchain: 1.98.1
        if: steps.base.outputs.exists == 'true'
        with:
          components: llvm-tools-preview
"""
        with self.assertRaisesRegex(yaml.constructor.ConstructorError, "duplicate mapping key: with"):
            load_unique(text)

    def test_nested_duplicate_permission_rejected(self):
        with self.assertRaisesRegex(yaml.constructor.ConstructorError, "duplicate mapping key: contents"):
            load_unique("jobs: {job: {permissions: {contents: read, contents: write}}}")

    def rust_step(self, inputs, action=RUST_ACTION):
        return {"jobs": {"job": {"steps": [{"uses": action, "with": inputs, "if": "true"}]}}}

    def test_unsupported_toolchain_input_rejected(self):
        errors = validate_rust_steps(self.rust_step({"toolchain": "1.98.1", "components": "rustfmt, clippy"}))
        self.assertTrue(any("unsupported input" in error for error in errors))

    def test_unknown_version_implementation_rejected(self):
        errors = validate_rust_steps(self.rust_step({"components": "rustfmt, clippy"},
                                                  "dtolnay/rust-toolchain@" + "a" * 40))
        self.assertTrue(any("reviewed 1.98.1" in error for error in errors))

    def test_missing_repository_component_rejected(self):
        for components in ("", "clippy", "rustfmt", "${{ matrix.components }}", ["rustfmt", "clippy"]):
            with self.subTest(components=components):
                errors = validate_rust_steps(self.rust_step({"components": components}))
                self.assertTrue(any("during setup" in error for error in errors))

    def test_conditional_single_mapping_accepted(self):
        workflow = self.rust_step({"components": "llvm-tools-preview, clippy, rustfmt",
                                   "targets": "aarch64-unknown-linux-gnu"})
        self.assertEqual(validate_rust_steps(workflow), [])


if __name__ == "__main__":
    unittest.main()
