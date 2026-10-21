#!/usr/bin/env python3
"""Negative coverage for real native/image acceptance and pinned driver inputs."""
import copy
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import shlex
import stat
import tarfile
import tempfile
import unittest

from workflow_yaml import load_unique

ROOT = Path(__file__).resolve().parents[1]


def module(name):
    spec = importlib.util.spec_from_file_location(name.replace('-', '_'), ROOT / 'scripts' / (name + '.py'))
    result = importlib.util.module_from_spec(spec); spec.loader.exec_module(result)
    return result


cua = module('install-pinned-cua')
prereqs = module('check-machine-native-prereqs')


def workflows():
    return {name: load_unique((ROOT / '.github/workflows' / (name + '.yml')).read_text())
            for name in ('ci', 'machine-native-acceptance')}


def step(job, text):
    found = [s for s in job['steps'] if text in s.get('run', '')]
    if len(found) != 1:
        raise ValueError('missing/duplicated required acceptance step: ' + text)
    result = found[0]
    if result.get('continue-on-error') or 'if' in result or any(t in result['run'] for t in ('|| true', 'set +e', '::warning::')):
        raise ValueError('acceptance gates must be mandatory: ' + text)
    return result['run']


def require(data):
    ci = data['ci']; workflow = data['machine-native-acceptance']
    caller = ci['jobs']['machine-native']
    if (caller.get('if') or caller.get('continue-on-error') or
            caller.get('uses') != './.github/workflows/machine-native-acceptance.yml' or
            caller.get('permissions') != {'contents': 'read'} or
            'machine-native' not in ci['jobs']['ci-pass']['needs'] or
            'machine-native=${{ needs.machine-native.result }}' not in str(ci['jobs']['ci-pass']['steps'])):
        raise ValueError('PR CI Pipeline must require native acceptance without secrets')
    if 'pull_request' not in ci.get('on', ci.get(True, {})):
        raise ValueError('native acceptance must be PR visible')
    events = workflow.get('on', workflow.get(True, {}))
    if not {'workflow_call', 'schedule'}.issubset(events) or workflow.get('permissions') != {'contents': 'read'}:
        raise ValueError('read-only PR and scheduled native acceptance required')
    native = workflow['jobs']['native']
    if (native.get('runs-on') != 'ubuntu-24.04' or native.get('container') or native.get('if') or
            native.get('continue-on-error') or 'secrets' in native or 'id-token' in str(workflow)):
        raise ValueError('native acceptance requires actual non-skippable Ubuntu 24.04 runner')
    body = str(native['steps'])
    if any(t in body for t in ('--no-sandbox', 'sysctl -w', 'apparmor=0', 'systemctl stop apparmor', '--probes-only')):
        raise ValueError('native acceptance cannot weaken the sandbox or shorten the harness')
    step(native, 'scripts/test-machine-acceptance-ci.py')
    install = step(native, 'scripts/apt-snapshot-install.sh --install-only security/apt-snapshots.tsv ubuntu:noble')
    for package in ('libnss3-tools', 'xvfb', 'openbox', 'dbus-x11', 'bubblewrap', 'at-spi2-core', 'python3-venv',
                    'python3-pip', 'cargo', 'rustc', 'nodejs', 'npm'):
        if package not in shlex.split(install.replace('\\\n', ' ')):
            raise ValueError('missing snapshot native/adversarial dependency: ' + package)
    step(native, 'scripts/install-pinned-cua.py --destination "$RUNNER_TEMP/machine-native-cua"')
    prepare = step(native, 'scripts/check-machine-native-prereqs.py --prepare-disposable-host')
    if 'sudo -n env ' not in prepare:
        raise ValueError('disposable managed ancestry preparation requires root')
    step(native, 'cargo build --locked -p nyxid-cli --bin nyxid')
    step(native, 'cargo build --locked --manifest-path cli/tests/machine_context_spike/Cargo.toml')
    run = step(native, 'bash cli/tests/machine_native_acceptance.sh --disposable')
    for text in ('set -euo pipefail', 'sudo -n env ',
                 'NYXID_TEST_CLI="$CARGO_TARGET_DIR/debug/nyxid"',
                 'NYXID_TEST_CUA_DRIVER="$RUNNER_TEMP/machine-native-cua/cua-driver"',
                 'NYXID_TEST_CONTEXT_SPIKE="$CARGO_TARGET_DIR/debug/nyxid-machine-context-spike"',
                 'NYXID_TEST_REQUIRE_APPARMOR_USERNS=1', 'NYXID_TEST_PROBE_SAMPLES=100',
                 'tee "$RUNNER_TEMP/machine-native-evidence/acceptance.log"'):
        if text not in run:
            raise ValueError('native acceptance lost root/harness/timing contract: ' + text)
    uploads = [s for s in native['steps'] if s.get('uses', '').startswith('actions/upload-artifact@')]
    if len(uploads) != 1 or uploads[0].get('if') != 'always()' or uploads[0]['with'].get('if-no-files-found') != 'error' or \
            uploads[0]['with'].get('path') != '${{ runner.temp }}/machine-native-evidence/probe-timings.txt':
        raise ValueError('fixed-label native timing artifact required even after failure')
    container = ci['jobs']['machine-container']
    if 'machine-container' not in ci['jobs']['ci-pass']['needs']:
        raise ValueError('full-image acceptance must gate CI Pipeline')
    run = step(container, '/test/machine_probe_acceptance.py --disposable')
    flat = run.replace('\\\n', ' ')
    command = next(line.strip() for line in flat.splitlines() if line.strip().startswith('docker run '))
    tokens = shlex.split(command.split('2>&1', 1)[0])
    expected = ['docker', 'run', '--rm', '--init', '--name', 'nyxid-machine-probe-acceptance', '--shm-size=512m',
                '--security-opt', 'seccomp=cli/resources/machine-container/seccomp.json', '--entrypoint', 'python3',
                'nyxid-machine-e2e:ci', '/test/machine_probe_acceptance.py', '--disposable', '--binary',
                '/usr/local/bin/nyxid', '--container', '--samples', '100']
    if tokens != expected or 'set -euo pipefail' not in run:
        raise ValueError('full-image timing requires exact init/shm/seccomp/100-sample command without credentials/mounts')
    if not all(text in run for text in ('volumes-before.txt', 'volumes-after.txt', 'diff -u ', 'test -z ', 'docker ps -aq')):
        raise ValueError('full-image timing must prove container and anonymous-volume cleanup')
    if not any(s.get('uses', '').startswith('actions/upload-artifact@') and s.get('if') == 'always()' and
               s.get('with', {}).get('path') == '${{ runner.temp }}/machine-container-evidence/probe-timings.txt' and
               s['with'].get('if-no-files-found') == 'error' for s in container['steps']):
        raise ValueError('actual full-image timing artifact required')


class AcceptanceWorkflows(unittest.TestCase):
    def test_current_contract_and_required_pr_graph(self):
        require(workflows())
        dockerfile = (ROOT / 'cli/tests/Dockerfile.machine').read_text()
        self.assertIn('COPY cli/tests/machine_probe_acceptance.py /test/', dockerfile)

    def test_pr_aggregator_schedule_native_container_or_optional_bypass_fails(self):
        for mutation in ('needs', 'result', 'schedule', 'container', 'runner', 'conditional', 'ignored'):
            data = workflows(); job = data['machine-native-acceptance']['jobs']['native']
            if mutation == 'needs': data['ci']['jobs']['ci-pass']['needs'].remove('machine-native')
            elif mutation == 'result':
                data['ci']['jobs']['ci-pass']['steps'][0]['env']['RESULTS'] = ''
            elif mutation == 'schedule':
                events = data['machine-native-acceptance'].get('on', data['machine-native-acceptance'].get(True)); del events['schedule']
            elif mutation == 'container': job['container'] = 'ubuntu:24.04'
            elif mutation == 'runner': job['runs-on'] = 'ubuntu-22.04'
            elif mutation == 'conditional': data['ci']['jobs']['machine-native']['if'] = 'false'
            else: job['continue-on-error'] = True
            with self.subTest(mutation=mutation), self.assertRaises(ValueError): require(data)

    def test_native_prerequisite_build_and_full_suite_cannot_be_skipped_or_shortened(self):
        for needle in ('check-machine-native-prereqs.py', 'install-pinned-cua.py', 'cargo build --locked -p',
                       'machine_native_acceptance.sh', 'test-machine-acceptance-ci.py'):
            for mutation in ('missing', 'conditional', 'ignored'):
                data = workflows(); steps = data['machine-native-acceptance']['jobs']['native']['steps']
                selected = next(s for s in steps if needle in s.get('run', ''))
                if mutation == 'missing': steps.remove(selected)
                elif mutation == 'conditional': selected['if'] = 'false'
                else: selected['continue-on-error'] = True
                with self.subTest(needle=needle, mutation=mutation), self.assertRaises(ValueError): require(data)
        for old, new in (('NYXID_TEST_PROBE_SAMPLES=100', 'NYXID_TEST_PROBE_SAMPLES=10'),
                         ('sudo -n env ', 'env '), ('set -euo pipefail', 'set -eu'),
                         ('--disposable', '--disposable --probes-only'),
                         ('NYXID_TEST_REQUIRE_APPARMOR_USERNS=1', 'NYXID_TEST_REQUIRE_APPARMOR_USERNS=0')):
            data = workflows(); steps = data['machine-native-acceptance']['jobs']['native']['steps']
            selected = next(s for s in steps if 'bash cli/tests/machine_native_acceptance.sh' in s.get('run', ''))
            selected['run'] = selected['run'].replace(old, new)
            with self.subTest(new=new), self.assertRaises(ValueError): require(data)
        for old, new in (('--prepare-disposable-host', ''), ('sudo -n env ', 'env ')):
            data = workflows(); steps = data['machine-native-acceptance']['jobs']['native']['steps']
            selected = next(s for s in steps if 'scripts/check-machine-native-prereqs.py' in s.get('run', ''))
            selected['run'] = selected['run'].replace(old, new)
            with self.subTest(preparation=new), self.assertRaises(ValueError): require(data)

    def test_full_image_cannot_add_token_mounts_or_weaken_settings(self):
        for old, new in (('--init ', ''), ('--shm-size=512m', '--shm-size=64m'),
                         ('--security-opt seccomp=', '--security-opt ignored='),
                         ('--samples 100', '--samples 10'), ('--rm ', '--rm -e NYXID_NODE_TOKEN=fixture '),
                         ('--rm ', '--rm --mount type=volume,target=/workspace '), ('set -euo pipefail', 'set -eu')):
            data = workflows(); steps = data['ci']['jobs']['machine-container']['steps']
            selected = next(s for s in steps if '/test/machine_probe_acceptance.py' in s.get('run', ''))
            selected['run'] = selected['run'].replace(old, new)
            with self.subTest(new=new), self.assertRaises(ValueError): require(data)

    def test_platform_abi_root_and_real_apparmor_restrictions_required(self):
        prereqs.require_platform('Linux', 'x86_64', 0, 6, '1')
        for args in (('Darwin', 'x86_64', 0, 6, '1'), ('Linux', 'aarch64', 0, 6, '1'),
                     ('Linux', 'x86_64', 1001, 6, '1'), ('Linux', 'x86_64', 0, -1, '1'),
                     ('Linux', 'x86_64', 0, 5, '1'), ('Linux', 'x86_64', 0, 6, '0')):
            with self.subTest(args=args), self.assertRaises(ValueError): prereqs.require_platform(*args)


class ManagedDirectories(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(); self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        (self.root / 'etc').mkdir(mode=0o755)
        self.policy = '/etc/opt/chrome/policies/managed'

    def prepare(self):
        prereqs.prepare_managed_directories(self.policy, self.root, os.getuid(), os.getgid())

    def check(self):
        prereqs.check_managed_directories(self.policy, self.root, os.getuid())

    def test_runner_image_world_writable_opt_is_repaired_without_recursing(self):
        # actions/runner-images ubuntu24/20261012.334 configure-system.sh:
        # chmod -R 777 /opt. This ancestor fails before Chrome policy setup.
        opt = self.root / 'opt'; opt.mkdir(); opt.chmod(0o777)
        unrelated = opt / 'unrelated-tool'; unrelated.mkdir(); unrelated.chmod(0o777)
        marker = unrelated / 'binary'; marker.write_text('do not change'); marker.chmod(0o777)
        with self.assertRaisesRegex(ValueError, r'unsafe native managed directory: /opt;'):
            self.check()
        system_before = {p: p.stat() for p in (self.root, self.root / 'etc')}
        self.prepare(); self.check()
        for directory in prereqs.managed_directories(self.policy):
            metadata = (self.root / directory.relative_to('/')).lstat()
            self.assertTrue(stat.S_ISDIR(metadata.st_mode))
            self.assertEqual(metadata.st_uid, os.getuid())
            self.assertEqual(metadata.st_mode & 0o022, 0)
        for path, before in system_before.items():
            self.assertEqual((path.stat().st_ino, path.stat().st_mode), (before.st_ino, before.st_mode))
        self.assertEqual(unrelated.stat().st_mode & 0o777, 0o777)
        self.assertEqual(marker.stat().st_mode & 0o777, 0o777)
        self.assertEqual(marker.read_text(), 'do not change')

    def test_each_group_or_other_writable_ancestor_is_reported_and_repaired(self):
        self.prepare()
        for directory in prereqs.managed_directories(self.policy):
            if directory in (Path('/'), Path('/etc')):
                continue
            path = self.root / directory.relative_to('/')
            for bit in (0o020, 0o002):
                with self.subTest(directory=str(directory), bit=bit):
                    path.chmod(0o755 | bit)
                    with self.assertRaisesRegex(ValueError, 'unsafe native managed directory: ' + str(directory) + ';'):
                        self.check()
                    self.prepare(); self.check()

    def test_symlinks_and_files_are_refused_without_touching_their_targets(self):
        self.prepare()
        target = self.root / 'untouched'; target.mkdir(); target.chmod(0o777)
        for directory in ('opt/nyxid/machine-browser/dev-policies', 'etc/opt/chrome/policies/managed',
                          'etc/opt/chrome/native-messaging-hosts'):
            path = self.root / directory; path.rmdir()
            for kind in ('symlink', 'file'):
                with self.subTest(directory=directory, kind=kind):
                    if kind == 'symlink': path.symlink_to(target, target_is_directory=True)
                    else: path.write_text('not a directory')
                    with self.assertRaises((OSError, ValueError)): self.prepare()
                    with self.assertRaises(ValueError): self.check()
                    self.assertEqual(target.stat().st_mode & 0o777, 0o777)
                    path.unlink()
            self.prepare()

    def test_untrusted_system_ancestors_and_wrong_owner_fail_closed(self):
        for path in (self.root, self.root / 'etc'):
            before = path.stat().st_mode
            try:
                path.chmod(0o777)
                with self.assertRaisesRegex(ValueError, 'unsafe system ancestor'): self.prepare()
                self.assertEqual(path.stat().st_mode & 0o777, 0o777)
            finally: path.chmod(before & 0o7777)
        self.prepare()
        with self.assertRaisesRegex(ValueError, 'unsafe native managed directory'):
            prereqs.check_managed_directories(self.policy, self.root, os.getuid() + 1)

    def test_diagnostics_include_identity_and_every_ancestor_with_missing_or_symlink_details(self):
        from contextlib import redirect_stdout
        self.prepare()
        leaf = self.root / 'etc/opt/chrome/policies/managed'; leaf.rmdir(); leaf.symlink_to('missing')
        output = io.StringIO()
        with redirect_stdout(output): prereqs.diagnose_managed_directories(self.policy, self.root)
        self.assertIn(f'euid={os.geteuid()}', output.getvalue())
        for directory in prereqs.managed_directories(self.policy):
            self.assertIn(f'path={directory} uid=', output.getvalue())
        mode = stat.S_IMODE(leaf.lstat().st_mode)
        self.assertIn(f'path={self.policy} uid={os.getuid()} gid={os.getgid()} mode={mode:04o} type=l', output.getvalue())
        leaf.unlink()
        output = io.StringIO()
        with redirect_stdout(output): prereqs.diagnose_managed_directories(self.policy, self.root)
        self.assertIn(f'path={self.policy} error=', output.getvalue())

    def test_supported_policy_layouts_prepare_only_their_native_host_paths(self):
        for policy in prereqs.POLICY_DIRECTORIES:
            with self.subTest(policy=policy):
                prereqs.prepare_managed_directories(policy, self.root, os.getuid(), os.getgid())
                prereqs.check_managed_directories(policy, self.root, os.getuid())
                native = ('etc/opt/chrome/native-messaging-hosts' if '/opt/chrome/' in policy
                          else 'etc/chromium/native-messaging-hosts')
                self.assertTrue((self.root / native).is_dir())


class PinnedDriver(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(); self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name); self.archive = self.root / 'driver.tar.gz'
        self.destination = self.root / 'installed'
        self.release = json.loads((ROOT / 'cli/resources/cua/release.json').read_text())
        self.asset = copy.deepcopy(cua.asset_for_host(self.release, 'Linux', 'x86_64'))
        self.prefix = self.asset['name'].removesuffix('.tar.gz')

    def archive_with(self, extra=None):
        with tarfile.open(self.archive, 'w:gz') as package:
            for name in ('cua-driver', 'libcua_driver_sdk.so'):
                entry = tarfile.TarInfo(self.prefix + '/' + name); entry.mode = 0o755
                data = b'reviewed test artifact'; entry.size = len(data); package.addfile(entry, io.BytesIO(data))
            if extra: package.addfile(extra)
        self.asset['size'] = self.archive.stat().st_size
        self.asset['sha256'] = hashlib.sha256(self.archive.read_bytes()).hexdigest()

    def test_exact_driver_and_sdk_extracted_without_install_execution(self):
        self.archive_with(); cua.unpack(self.archive, self.asset, self.destination)
        self.assertEqual({p.name for p in self.destination.iterdir()}, {'cua-driver', 'libcua_driver_sdk.so'})
        self.assertEqual((self.destination / 'cua-driver').stat().st_mode & 0o777, 0o755)

    def test_corruption_and_wrong_size_refused_before_extraction(self):
        for mutation in ('digest', 'size'):
            self.archive_with()
            if mutation == 'digest': self.asset['sha256'] = '0' * 64
            else: self.asset['size'] += 1
            with self.subTest(mutation=mutation), self.assertRaisesRegex(ValueError, 'size/checksum'): cua.unpack(self.archive, self.asset, self.destination)
            self.assertFalse(self.destination.exists())

    def test_traversal_and_symlink_refused_before_extraction(self):
        for mutation in ('traversal', 'absolute', 'symlink'):
            entry = tarfile.TarInfo(self.prefix + '/../escape' if mutation == 'traversal' else '/escape' if mutation == 'absolute' else self.prefix + '/link')
            if mutation == 'symlink': entry.type = tarfile.SYMTYPE; entry.linkname = '/etc/passwd'
            self.archive_with(entry)
            with self.subTest(mutation=mutation), self.assertRaisesRegex(ValueError, 'unsafe'): cua.unpack(self.archive, self.asset, self.destination)
            self.assertFalse(self.destination.exists())

    def test_unpinned_url_architecture_duplicate_or_missing_hash_fails(self):
        for mutation in ('http', 'origin', 'duplicate', 'hash', 'arch'):
            release = copy.deepcopy(self.release)
            asset = next(a for a in release['assets'] if a['name'] == self.asset['name'])
            if mutation == 'http': asset['url'] = asset['url'].replace('https:', 'http:')
            elif mutation == 'origin': asset['url'] = asset['url'].replace('github.com', 'attacker.invalid')
            elif mutation == 'duplicate': release['assets'].append(asset)
            elif mutation == 'hash': asset['sha256'] = ''
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                cua.asset_for_host(release, 'Linux', 'unsupported' if mutation == 'arch' else 'x86_64')


if __name__ == '__main__':
    unittest.main()
