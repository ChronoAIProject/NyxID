import os
from pathlib import Path
import subprocess
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("install-linux-build-deps.sh").resolve()


class BuildDependencyTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="ci dependencies ")
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.calls = self.root / "calls"
        self.harness = self.root / "commands.sh"
        # Exercise failure propagation without modifying the developer's system.
        self.harness.write_text('''
sudo() {
  [[ "$1" == -n && "$2" == env && "$3" == DEBIAN_FRONTEND=noninteractive && "$4" == apt-get ]] || return 99
  shift 4
  local operation arg
  for arg in "$@"; do
    case "$arg" in update|install) operation="$arg";; esac
  done
  printf '%s\\n' "$operation" >> "$TEST_CALLS"
  if [[ "$operation" == "$TEST_FAIL" ]]; then
    return 17
  fi
}
''')

    def invoke(self, failure=""):
        return subprocess.run(
            ["bash", str(SCRIPT)], env={**os.environ, "BASH_ENV": str(self.harness),
                "TEST_CALLS": str(self.calls), "TEST_FAIL": failure},
            capture_output=True, text=True, timeout=10,
        )

    def test_success_requires_update_and_install(self):
        result = self.invoke()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.calls.read_text().splitlines(), ["update", "install"])

    def test_update_failure_stops_before_install(self):
        result = self.invoke("update")
        self.assertEqual(result.returncode, 17, result.stderr)
        self.assertEqual(self.calls.read_text().splitlines(), ["update"])

    def test_install_failure_is_not_ignored(self):
        result = self.invoke("install")
        self.assertEqual(result.returncode, 17, result.stderr)
        self.assertEqual(self.calls.read_text().splitlines(), ["update", "install"])


if __name__ == "__main__":
    unittest.main()
