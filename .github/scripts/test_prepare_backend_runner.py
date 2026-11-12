import os
from pathlib import Path
import subprocess
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("prepare-backend-runner.sh").resolve()
RESERVE_KIB = 40 * 1024**2


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
  if [[ "$1" == swapoff ]]; then
    [[ "$TEST_FAIL" != release ]] || return 1
    [[ -f "$2" ]] || return 2
  fi
}
''')

    def invoke(self, initial=3 * 1024**2, disk=80 * 1024**2, fail="", result=RESERVE_KIB, args=()):
        (self.root / "swap-state").write_text(str(initial))
        return subprocess.run(
            ["bash", str(SCRIPT), *args], env={**os.environ, "BASH_ENV": str(self.harness),
                "RUNNER_TEMP": str(self.root), "TEST_DISK_KIB": str(disk),
                "TEST_FAIL": fail, "TEST_RESULT_KIB": str(result)},
            capture_output=True, text=True, timeout=10,
        )

    def test_existing_capacity_needs_no_privileged_commands(self):
        result = self.invoke(initial=RESERVE_KIB)
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
        result = self.invoke(result=8 * 1024**2)
        self.assertNotEqual(result.returncode, 0)
        # Do not remove a potentially active file after a verification failure.
        self.assertEqual(len(list(self.root.glob("nyxid-ci-swap.*"))), 1)


    def test_release_disables_owned_swap_before_removing_it(self):
        self.assertEqual(self.invoke().returncode, 0)
        record = self.root / "nyxid-ci-swap-path"
        allocated = Path(record.read_text().strip())
        unrelated = self.root / "runner-swap"
        unrelated.write_text("do not remove")
        released = self.invoke(args=("--release",))
        self.assertEqual(released.returncode, 0, released.stderr)
        self.assertIn(f"swapoff {allocated}", (self.root / "calls").read_text())
        self.assertFalse(allocated.exists())
        self.assertFalse(record.exists())
        self.assertEqual(unrelated.read_text(), "do not remove")
        self.assertEqual(self.invoke(args=("--release",)).returncode, 0)

    def test_failed_swapoff_keeps_active_file_and_record(self):
        self.assertEqual(self.invoke().returncode, 0)
        record = self.root / "nyxid-ci-swap-path"
        allocated = Path(record.read_text().strip())
        self.assertNotEqual(self.invoke(args=("--release",), fail="release").returncode, 0)
        self.assertTrue(allocated.exists())
        self.assertTrue(record.exists())

    def test_release_without_owned_reserve_does_nothing(self):
        self.assertEqual(self.invoke(initial=RESERVE_KIB).returncode, 0)
        self.assertEqual(self.invoke(args=("--release",)).returncode, 0)
        self.assertFalse((self.root / "calls").exists())

    def test_invalid_release_record_does_not_invoke_swapoff(self):
        unrelated = self.root / "runner-swap"
        unrelated.write_text("keep")
        record = self.root / "nyxid-ci-swap-path"
        record.write_text(str(unrelated))
        self.assertNotEqual(self.invoke(args=("--release",)).returncode, 0)
        self.assertTrue(unrelated.exists())
        self.assertFalse((self.root / "calls").exists())


if __name__ == "__main__":
    unittest.main()
