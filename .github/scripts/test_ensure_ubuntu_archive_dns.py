import os
from pathlib import Path
import subprocess
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("ensure-ubuntu-archive-dns.sh").resolve()


class ArchiveDnsFallbackTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="ci dns ")
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.calls = self.root / "calls"
        self.calls.write_text("")
        self.harness = self.root / "commands.sh"
        # Never touch the host resolver: every external command is replaced.
        self.harness.write_text('''
timeout() { shift; "$@"; }
getent() {
  if [[ "$TEST_RESOLVES" == always ]]; then return 0; fi
  if [[ "$TEST_RESOLVES" == after ]] && grep -q '^dns ' "$TEST_CALLS"; then return 0; fi
  return 2
}
resolvectl() {
  [[ "$TEST_RESOLVECTL" == present ]] || return 127
  [[ "$1" == dns && $# -eq 1 ]] || return 99
  printf '%s\\n' "$TEST_LINKS"
}
sudo() {
  [[ "$1" == -n && "$2" == resolvectl ]] || return 99
  shift 2
  printf '%s\\n' "$*" >> "$TEST_CALLS"
  [[ "$TEST_SUDO" == ok ]]
}
command() {
  if [[ "$1" == -v && "$2" == resolvectl ]]; then [[ "$TEST_RESOLVECTL" == present ]]; return; fi
  builtin command "$@"
}
''')

    def invoke(self, resolves="never", resolvectl="present", sudo="ok", skip="",
               links="Global:\\nLink 2 (eth0): 168.63.129.16\\nLink 3 (docker0):"):
        return subprocess.run(
            ["bash", str(SCRIPT)],
            env={**os.environ, "BASH_ENV": str(self.harness), "TEST_CALLS": str(self.calls),
                 "TEST_RESOLVES": resolves, "TEST_RESOLVECTL": resolvectl, "TEST_SUDO": sudo,
                 "TEST_LINKS": links.replace("\\n", "\n"), "NYXID_CI_SKIP_DNS_FALLBACK": skip},
            capture_output=True, text=True, timeout=10,
        )

    def changes(self):
        return self.calls.read_text().splitlines()

    def test_working_dns_is_left_alone(self):
        result = self.invoke(resolves="always")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.changes(), [])
        self.assertNotIn("warning", result.stdout)

    def test_failed_lookup_puts_public_resolvers_before_the_runner_server(self):
        result = self.invoke(resolves="after")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.changes(), ["dns eth0 1.1.1.1 9.9.9.9 168.63.129.16", "flush-caches"])
        self.assertIn("Public resolvers answer", result.stdout)

    def test_every_soft_failure_exits_zero_without_guessing(self):
        for options in ({"resolvectl": "absent"}, {"links": "Global:\\nLink 2 (eth0):"},
                        {"sudo": "denied"}, {"resolves": "never"}):
            with self.subTest(options=options):
                self.calls.write_text("")
                result = self.invoke(**options)
                self.assertEqual(result.returncode, 0, result.stderr)
        self.calls.write_text("")
        self.invoke(resolvectl="absent")
        self.assertEqual(self.changes(), [])

    def test_opt_out_skips_every_command(self):
        result = self.invoke(skip="1")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.changes(), [])
        self.assertEqual(result.stdout, "")


if __name__ == "__main__":
    unittest.main()
