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
        self.arguments = self.root / "arguments"
        self.sources = self.root / "sources"
        self.sources_path = self.root / "sources-path"
        self.harness = self.root / "commands.sh"
        # Exercise failure propagation without modifying the developer's system.
        self.harness.write_text('''
source() {
  [[ "$1" == /etc/os-release ]] || return 99
  ID="$TEST_DISTRO"
  VERSION_CODENAME="$TEST_CODENAME"
}
dpkg() {
  [[ "$1" == --print-architecture ]] || return 99
  printf '%s\\n' "$TEST_ARCH"
}
sudo() {
  [[ "$1" == -n && "$2" == env && "$3" == DEBIAN_FRONTEND=noninteractive && "$4" == apt-get ]] || return 99
  shift 4
  printf '%s\\n' "$@" >> "$TEST_ARGUMENTS"
  local operation arg
  for arg in "$@"; do
    case "$arg" in
      update|install) operation="$arg";;
      Dir::Etc::sourcelist=*)
        local sources="${arg#*=}"
        [[ -r "$sources" ]] || return 98
        cat "$sources" >> "$TEST_SOURCES"
        printf '%s\\n' "$sources" >> "$TEST_SOURCES_PATH"
        ;;
    esac
  done
  printf '%s\\n' "$operation" >> "$TEST_CALLS"
  if [[ "$operation" == "$TEST_FAIL" ]]; then
    return 17
  fi
}
''')

    def invoke(self, failure="", distro="ubuntu", codename="noble", arch="amd64"):
        return subprocess.run(
            ["bash", str(SCRIPT)], env={**os.environ, "BASH_ENV": str(self.harness),
                "TEST_CALLS": str(self.calls), "TEST_ARGUMENTS": str(self.arguments), "TEST_FAIL": failure,
                "TEST_DISTRO": distro, "TEST_CODENAME": codename, "TEST_ARCH": arch,
                "TEST_SOURCES": str(self.sources), "TEST_SOURCES_PATH": str(self.sources_path)},
            capture_output=True, text=True, timeout=10,
        )

    def test_success_requires_update_and_install(self):
        result = self.invoke()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.calls.read_text().splitlines(), ["update", "install"])

    def test_install_includes_backend_boundary_scanner(self):
        result = self.invoke()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("ripgrep", self.arguments.read_text().splitlines())

    def test_both_commands_use_direct_signed_ubuntu_sources(self):
        result = self.invoke()
        self.assertEqual(result.returncode, 0, result.stderr)
        sources = self.sources.read_text()
        self.assertEqual(sources.count("URIs: https://archive.ubuntu.com/ubuntu\n"), 2)
        self.assertEqual(sources.count("URIs: https://security.ubuntu.com/ubuntu\n"), 2)
        self.assertIn("Suites: noble noble-updates noble-backports\n", sources)
        self.assertIn("Suites: noble-security\n", sources)
        self.assertEqual(sources.count("Signed-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg\n"), 4)
        arguments = self.arguments.read_text().splitlines()
        self.assertEqual(arguments.count("Dir::Etc::sourceparts=-"), 2)
        self.assertEqual(arguments.count("APT::Update::Error-Mode=any"), 2)
        self.assertEqual(arguments.count("Acquire::http::Timeout=20"), 2)
        self.assertEqual(arguments.count("Acquire::https::Timeout=20"), 2)
        self.assertNotIn("mirror+file:", sources)

    def test_ubuntu_ports_use_direct_signed_repositories(self):
        result = self.invoke(codename="jammy", arch="arm64")
        self.assertEqual(result.returncode, 0, result.stderr)
        sources = self.sources.read_text()
        self.assertEqual(sources.count("URIs: https://ports.ubuntu.com/ubuntu-ports\n"), 4)
        self.assertIn("Suites: jammy-security\n", sources)

    def test_unsupported_os_fails_before_any_package_operation(self):
        for distro, codename in (("debian", "bookworm"), ("ubuntu", ""), ("ubuntu", "noble\nextra")):
            with self.subTest(distro=distro, codename=codename):
                result = self.invoke(distro=distro, codename=codename)
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse(self.calls.exists())

    def test_temporary_sources_are_removed_on_success_and_failure(self):
        for failure in ("", "update", "install"):
            with self.subTest(failure=failure):
                result = self.invoke(failure)
                self.assertEqual(result.returncode, 17 if failure else 0, result.stderr)
                for path in self.sources_path.read_text().splitlines():
                    self.assertFalse(Path(path).parent.exists())

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
