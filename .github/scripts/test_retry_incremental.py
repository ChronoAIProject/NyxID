import unittest

from retry_incremental import should_retry


class RetryIncrementalTests(unittest.TestCase):
    def test_internal_compiler_error_retries_case_insensitively(self):
        self.assertTrue(should_retry("error: Internal Compiler Error: stale query"))

    def test_incremental_markers_retry(self):
        for output in (
            "error: incremental compilation failure",
            "failed to load dep graph: metadata is corrupt",
            "could not read dep-graph file",
        ):
            with self.subTest(output=output):
                self.assertTrue(should_retry(output))

    def test_ordinary_compile_error_does_not_retry(self):
        self.assertFalse(should_retry("error[E0308]: mismatched types"))

    def test_test_failure_does_not_retry(self):
        self.assertFalse(should_retry("test foo ... FAILED\nthread panicked at assertion"))


if __name__ == "__main__":
    unittest.main()
