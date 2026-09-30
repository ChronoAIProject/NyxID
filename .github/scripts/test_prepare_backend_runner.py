import os
from pathlib import Path
import subprocess
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("prepare-backend-runner.sh").resolve()


class BackendReserveTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="ci reserve ")
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.harness = self.root / "commands.sh"
        # Exercise the real setup script without altering the developer's swap.
        self.harness.write_text('''
awk() {
  if [[ "$*" == *"/proc/meminfo"* ]]; then
    cat "$RUNNER_TEMP/swap-state"
  else
    command awk "$@"
  fi
}
df() { printf 'Filesystem 1024-blocks Used Available Capacity Mounted\nfake 999999999 0 %s 0%% /\n' "$TEST_DISK_KIB"; }
free() { :; }
fallocate() {
  printf 'allocate %s\n' "$*" >> "$RUNNER_TEMP/calls"
  [[ "$TEST_FAIL" != allocation ]]
}
sudo() {
  [[ "$1" == -n ]] || return 99
  shift
  printf '%s\n' "$*" >> "$RUNNER_TEMP/calls"
  if [[ "$1" == swapon ]]; then
    [[ "$TEST_FAIL" != activation ]] || return 1
    printf '%s\n' "$TEST_RESULT_KIB" > "$RUNNER_TEMP/swap-state"
  fi
}
''')

    def invoke(self, initial=3 * 1024**2, disk=80 * 1024**2, fail="", result=8 * 1024**2):
        (self.root / "swap-state").write_text(str(initial))
        return subprocess.run(
            ["bash", str(SCRIPT)], env={**os.environ, "BASH_ENV": str(self.harness),
                "RUNNER_TEMP": str(self.root), "TEST_DISK_KIB": str(disk),
                "TEST_FAIL": fail, "TEST_RESULT_KIB": str(result)},
            capture_output=True, text=True, timeout=10,
        )

    def test_existing_capacity_needs_no_privileged_commands(self):
        result = self.invoke(initial=8 * 1024**2)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse((self.root / "calls").exists())

    def test_adds_reserve_and_retains_private_activated_file(self):
        result = self.invoke()
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = (self.root / "calls").read_text()
        self.assertIn("mkswap", calls)
        self.assertIn("swapon", calls)
        files = list(self.root.glob("nyxid-ci-swap.*"))
        self.assertEqual(len(files), 1)
        self.assertEqual(files[0].stat().st_mode & 0o777, 0o600)

    def test_insufficient_disk_fails_before_allocation(self):
        result = self.invoke(disk=10 * 1024**2)
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse((self.root / "calls").exists())

    def test_failed_preparation_removes_only_unactivated_file(self):
        for failure in ("allocation", "activation"):
            with self.subTest(failure=failure):
                result = self.invoke(fail=failure)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(list(self.root.glob("nyxid-ci-swap.*")), [])

    def test_successful_swapon_with_insufficient_actual_capacity_fails(self):
        result = self.invoke(result=3 * 1024**2)
        self.assertNotEqual(result.returncode, 0)
        # Do not remove a potentially active file after a verification failure.
        self.assertEqual(len(list(self.root.glob("nyxid-ci-swap.*"))), 1)


if __name__ == "__main__":
    unittest.main()
