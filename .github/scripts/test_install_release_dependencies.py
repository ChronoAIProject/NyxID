"""Exercise release/cache prerequisites with the real signed-snapshot helper."""
import unittest

from test_install_linux_build_deps import BuildDependencyHarness, ROOT


class ReleaseDependencyTests(BuildDependencyHarness, unittest.TestCase):
    source = ROOT / 'scripts/install-release-dependencies.sh'

    def setUp(self):
        super().setUp()
        self.stub('uname', 'echo Linux\n')
        # The release helper execs sudo, so provide an executable fixture too
        # (shell functions alone do not intercept exec).
        self.stub('sudo', '''[ "$1" = -n ] && [ "$2" = sh ] &&
[ "$3" = "$TEST_ROOT/scripts/apt-snapshot-install.sh" ] &&
[ "$4" = --install-only ] && [ "$5" = "$TEST_ROOT/scripts/../security/apt-snapshots.tsv" ] &&
[ "$6" = ubuntu:noble ] && [ "$7" = -- ] || exit 99
shift
export TEST_SUDO=1
exec "$@"
''')

    def test_minimal_root_container_installs_every_release_tool_from_snapshot(self):
        result = self.invoke()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.calls.read_text().splitlines(),
                         ['update', 'download-upgrade', 'upgrade', 'download', 'install'])
        for package in ('clang', 'jq', 'libdbus-1-dev', 'pkg-config', 'python3'):
            with self.subTest(package=package):
                self.assertEqual(self.arguments.read_text().splitlines().count(package), 2)

    def test_nonroot_release_runner_uses_same_authenticated_offline_install(self):
        self.stub('id', 'if [ "${TEST_SUDO:-}" = 1 ]; then echo 0; else echo 1001; fi\n')
        result = self.invoke()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.calls.read_text().splitlines(), ['update', 'download', 'install'])
        self.assertEqual(self.arguments.read_text().splitlines().count('jq'), 2)

    def test_release_update_download_and_install_failures_remain_blocking(self):
        phases = ['update', 'download-upgrade', 'upgrade', 'download', 'install']
        for index, phase in enumerate(phases):
            with self.subTest(phase=phase):
                self.calls.unlink(missing_ok=True)
                result = self.invoke(phase)
                self.assertEqual(result.returncode, 17, result.stderr)
                self.assertEqual(self.calls.read_text().splitlines(), phases[:index + 1])


if __name__ == '__main__':
    unittest.main()
