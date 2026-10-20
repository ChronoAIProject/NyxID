#!/usr/bin/env python3
"""Exercise the real installer with isolated package/filesystem command fixtures.

Native PR replay supplies real APK crypto and package transactions. These tests
prove installer ordering when the solver accepts an incomplete closure, and
that neither a failed scratch transaction nor a wrong inventory touches the
live package state.
"""
import hashlib
import importlib.util
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]


def sha(data):
    return hashlib.sha256(data).hexdigest()


def database(version):
    return f'P:fixture\nV:{version}\nA:x86_64\n\n'.encode()


class Preflight(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix='apk preflight ')
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        for name in ('bin', 'live/lib/apk/db', 'live/etc/apk', 'archives', 'scripts'):
            (self.root / name).mkdir(parents=True)
        self.live = self.root / 'live/lib/apk/db/installed'
        self.live.write_bytes(database('1-r0'))
        self.calls = self.root / 'calls'
        source = (ROOT / 'scripts/apk-input-install.sh').read_text()
        # Redirect filesystem boundaries, keeping every control/integrity check.
        for old, new in (
            ('/lib/apk', str(self.root / 'live/lib/apk')),
            ('/etc/apk', str(self.root / 'live/etc/apk')),
        ):
            # Scratch-root paths must retain their native internal layout.
            source = source.replace(old, new).replace('$work/root' + new, '$work/root' + old)
            source = source.replace('cp -R ' + new + ' ', 'cp -R "' + new + '" ')
        source = source.replace('/usr/share/nyxid/build-inputs/apk/', '"' + str(self.root / 'receipts') + '"/')
        self.installer = self.root / 'scripts/apk-input-install.sh'
        self.installer.write_text(source)
        for name in ('apk-catalog.awk', 'apk-inventory.awk'):
            shutil.copyfile(ROOT / 'scripts' / name, self.root / 'scripts' / name)
        self.stub('id', '#!/bin/sh\nprintf "0\\n"\n')
        self.stub('tar', '#!/bin/sh\nprintf "pkgname = fixture\\npkgver = 2-r0\\narch = x86_64\\n"\n')
        self.stub('apk', '''#!/usr/bin/env python3
import os, sys
from pathlib import Path
args = sys.argv[1:]
root = Path(os.environ['TEST_ROOT'])
if args == ['--print-arch']:
    print('x86_64')
elif args[0] == 'verify':
    pass
elif 'add' in args:
    scratch = '--root' in args
    with (root / 'calls').open('a') as output:
        output.write(('scratch' if scratch else 'live') + '\\n')
    assert '--no-network' in args and '--no-cache' in args
    assert args[args.index('--repositories-file') + 1] == '/dev/null'
    if scratch:
        assert '--no-scripts' in args
        scratch_root = Path(args[args.index('--root') + 1])
        # APK 2 requires these parents even when --no-cache is set.
        assert (scratch_root / 'var/cache/apk').is_dir()
        assert not list((scratch_root / 'var/cache/apk').iterdir())
        target = scratch_root / 'lib/apk/db/installed'
        # Confirm the scratch transaction starts from the actual base DB.
        assert target.read_bytes() == (root / 'live/lib/apk/db/installed').read_bytes()
        if os.environ['CASE'] == 'solver-failure':
            sys.exit(17)
        version = '1-r0' if os.environ['CASE'] == 'incomplete' else '2-r0'
    else:
        assert '--no-scripts' not in args
        target = root / 'live/lib/apk/db/installed'
        version = '3-r0' if os.environ['CASE'] == 'post-install-drift' else '2-r0'
    target.write_text('P:fixture\\nV:' + version + '\\nA:x86_64\\n\\n')
else:
    sys.exit(99)
''')
        archive = self.root / 'archives/fixture-2-r0.apk'
        archive.write_bytes(b'signed archive fixture (native CI verifies real crypto)')
        checksum = sha(archive.read_bytes())
        self.base = 'alpine:3.22@sha256:' + 'a' * 64
        self.catalog = self.root / 'catalog.tsv'
        before = sha(b'fixture\t1-r0\tx86_64\n')
        after = sha(b'fixture\t2-r0\tx86_64\n')
        self.catalog.write_text(f'''# nyxid.apk-inputs.v2
# base-image {self.base}
# arch x86_64
# refreshed-on 2026-10-19
# packages fixture
# before-sha256 {before}
# after-sha256 {after}
fixture\t2-r0\tx86_64\t{checksum}\thttps://github.com/ChronoAIProject/NyxID/releases/download/build-inputs-apk-2026-10-19/{checksum}-x86_64-fixture-2-r0.apk\thttps://dl-cdn.alpinelinux.org/alpine/v3.22/main/x86_64/fixture-2-r0.apk
''')

    def stub(self, name, source):
        target = self.root / 'bin' / name
        target.write_text(source)
        target.chmod(0o755)

    def invoke(self, case):
        return subprocess.run(['sh', str(self.installer), '--install', str(self.catalog), self.base,
                               str(self.root / 'archives')], capture_output=True, text=True,
                              env={**os.environ, 'PATH': str(self.root / 'bin') + os.pathsep + os.environ['PATH'],
                                   'TEST_ROOT': str(self.root), 'CASE': case}, timeout=10)

    def test_success_checks_scratch_before_live_install(self):
        result = self.invoke('success')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.calls.read_text().splitlines(), ['scratch', 'live'])
        self.assertEqual(self.live.read_bytes(), database('2-r0'))
        self.assertEqual(len(list((self.root / 'receipts').iterdir())), 1)

    def test_solver_accepts_incomplete_closure_but_inventory_rejects_before_mutation(self):
        result = self.invoke('incomplete')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('APK closure preflight inventory mismatch', result.stderr)
        self.assertEqual(self.calls.read_text().splitlines(), ['scratch'])
        self.assertEqual(self.live.read_bytes(), database('1-r0'))
        self.assertFalse((self.root / 'receipts').exists())

    def test_failed_scratch_transaction_leaves_live_database_unchanged(self):
        result = self.invoke('solver-failure')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('APK closure preflight failed', result.stderr)
        self.assertEqual(self.calls.read_text().splitlines(), ['scratch'])
        self.assertEqual(self.live.read_bytes(), database('1-r0'))
        self.assertFalse((self.root / 'receipts').exists())

    def test_wrong_catalog_final_inventory_fails_before_mutation(self):
        self.catalog.write_text(self.catalog.read_text().replace(sha(b'fixture\t2-r0\tx86_64\n'), '0' * 64))
        result = self.invoke('success')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('APK closure preflight inventory mismatch', result.stderr)
        self.assertEqual(self.calls.read_text().splitlines(), ['scratch'])
        self.assertEqual(self.live.read_bytes(), database('1-r0'))

    def test_real_transaction_still_requires_final_inventory(self):
        result = self.invoke('post-install-drift')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('APK final inventory mismatch', result.stderr)
        self.assertEqual(self.calls.read_text().splitlines(), ['scratch', 'live'])
        self.assertFalse((self.root / 'receipts').exists())


class ReplayOwnership(unittest.TestCase):
    """Execute the replay wrapper shell; only the Docker boundary is replaced."""
    def setUp(self):
        spec = importlib.util.spec_from_file_location('apk_replay', ROOT / 'scripts/test-apk-replay.py')
        self.replay = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.replay)
        temporary = tempfile.TemporaryDirectory(prefix='apk-replay-owner-')
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.folder = self.root / 'case'
        self.folder.mkdir()
        for name in ('input', 'lib/apk/db', 'bin', 'records'):
            (self.root / name).mkdir(parents=True)
        (self.root / 'lib/apk/db/installed').write_text('unchanged base DB')
        self.chowns = self.root / 'chowns'
        chown = self.root / 'bin/chown'
        chown.write_text('#!/bin/sh\nprintf "%s\\n" "$@" > "' + str(self.chowns) + '"\n')
        chown.chmod(0o755)

    def invoke(self, mode, expected, failure=False):
        if mode == '--install':
            (self.folder / 'archives').mkdir()
        installer = self.root / 'input/apk-input-install.sh'
        installer.write_text('#!/bin/sh\n' + (
            'mkdir -p "$4"\nprintf "verified fixture" > "$4/fixture.apk"\n' if mode == '--download' else '') +
            ('exit 1\n' if failure else 'exit 0\n'))
        real_run = subprocess.run

        def docker(command, **kwargs):
            self.assertEqual(command[:2], ['docker', 'run'])
            if mode == '--install':
                self.assertIn(str(self.folder / 'archives') + ':/case/archives:ro', command)
            shell = command[command.index('-ec') + 1]
            for original, replacement in (
                ('/lib/apk/db/installed', str(self.root / 'lib/apk/db/installed')),
                ('/usr/share/nyxid/build-inputs/apk', str(self.root / 'records')),
                ('/case', str(self.folder)), ('/input', str(self.root / 'input')),
            ):
                shell = shell.replace(original, replacement)
            return real_run(['sh', '-ec', shell, *command[command.index('-ec') + 2:]],
                            env={**os.environ, 'PATH': str(self.root / 'bin') + os.pathsep + os.environ['PATH']},
                            **kwargs)

        with mock.patch.object(self.replay.subprocess, 'run', side_effect=docker):
            self.replay.invoke('fixture-base', 'amd64', self.folder, mode, expected, self.root / 'run.log', 'offline')

    def test_successful_download_returns_directory_to_host_before_cleanup(self):
        self.invoke('--download', 'download-success')
        self.assertEqual(self.chowns.read_text().splitlines(),
                         ['-R', str(os.getuid()) + ':' + str(os.getgid()), str(self.folder / 'archives')])
        shutil.rmtree(self.folder / 'archives')

    def test_failed_download_also_returns_output_to_host(self):
        self.invoke('--download', 'failure', failure=True)
        self.assertTrue(self.chowns.is_file())
        shutil.rmtree(self.folder / 'archives')

    def test_install_does_not_chown_read_only_archives(self):
        self.invoke('--install', 'success')
        self.assertFalse(self.chowns.exists())


if __name__ == '__main__':
    unittest.main()
