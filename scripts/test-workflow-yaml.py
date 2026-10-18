#!/usr/bin/env python3
import unittest
import yaml
from workflow_yaml import load_unique, validate_rust_steps


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

    def test_missing_explicit_toolchain_rejected(self):
        workflow = load_unique("jobs: {job: {steps: [{uses: 'dtolnay/rust-toolchain@abc', with: {components: clippy}}]}}")
        self.assertEqual(len(validate_rust_steps(workflow)), 1)

    def test_conditional_single_mapping_accepted(self):
        workflow = load_unique("jobs: {job: {steps: [{uses: 'dtolnay/rust-toolchain@abc', if: 'true', with: {toolchain: 1.98.1, components: llvm-tools-preview}}]}}")
        self.assertEqual(validate_rust_steps(workflow), [])


if __name__ == "__main__":
    unittest.main()
