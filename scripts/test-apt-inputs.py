#!/usr/bin/env python3
"""Portable catalog, caller and real-test-harness rejection regressions."""
import importlib.util
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

from apt_inputs import INPUTS, INSTALLER, ROOTS, check_apt_inputs, projection, select, stages

ROOT = Path(__file__).resolve().parents[1]


def load(filename):
    spec = importlib.util.spec_from_file_location(filename, ROOT / 'scripts' / filename)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


harness = load('test-apt-snapshots.py')
images = load('test-apt-image-inputs.py')


class Catalog(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        for name in INPUTS:
            target = self.root / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / name, target)
        self.catalog = self.root / INPUTS[2]
        self.original = self.catalog.read_text()

    def reject(self, text, selectors=ROOTS):
        self.catalog.write_text(text)
        with self.assertRaises(ValueError):
            select(self.root, selectors)

    def test_all_four_complete_roots_and_mixed_selection(self):
        self.assertEqual(len(select(self.root, ROOTS).splitlines()), 12)
        self.assertEqual(len(select(self.root, ('ubuntu:noble', 'debian:bookworm')).splitlines()), 6)

    def test_missing_requested_root_with_another_valid_root(self):
        self.reject('\n'.join(line for line in self.original.splitlines() if '\tbookworm\t' not in line) + '\n',
                    ('ubuntu:noble', 'debian:bookworm'))

    def test_no_roots_duplicate_roots_and_unknown_roots(self):
        for roots in ((), ('ubuntu:noble', 'ubuntu:noble'), ('debian:sid',)):
            with self.subTest(roots=roots), self.assertRaises(ValueError):
                select(self.root, roots)

    def test_duplicate_suite_and_missing_suite(self):
        row = self.original.splitlines()[1]
        self.reject(self.original + row + '\n')
        self.reject(self.original.replace(row + '\n', ''))

    def test_bad_hash_rejected(self):
        row = self.original.splitlines()[1]
        for digest in ('0' * 63, 'z' * 64, 'A' * 64):
            with self.subTest(digest=digest):
                self.reject(self.original.replace(row, '\t'.join(row.split('\t')[:-1] + [digest])))

    def test_https_is_required_for_every_catalog_root(self):
        for root in ROOTS:
            distro, codename = root.split(':')
            row = next(line for line in self.original.splitlines() if line.startswith(distro + '\t' + codename + '\t'))
            with self.subTest(root=root):
                self.reject(self.original.replace(row, row.replace('https://', 'http://')))

    def test_origin_suffix_and_security_archive_rejected(self):
        for old, new in (('snapshot.debian.org/', 'snapshot.debian.org.evil.invalid/'),
                         ('archive/debian-security/', 'archive/debian/'),
                         ('https://snapshot.ubuntu.com/', 'http://snapshot.ubuntu.com/')):
            with self.subTest(new=new):
                self.reject(self.original.replace(old, new))

    def test_timestamp_mixed_root_and_path_injection_rejected(self):
        for replacement in ('20261018', '20261018T000000Z/extra', '20261018T000000Z?mutable=1'):
            with self.subTest(replacement=replacement):
                self.reject(self.original.replace('20261018T000000Z', replacement))
        self.reject(self.original.replace('20261018T000000Z', '20261017T000000Z', 1))

    def test_components_unknown_root_and_extra_field_rejected(self):
        self.reject(self.original.replace('main,universe', 'main'))
        self.reject(self.original.replace('\tfocal\t', '\tjammy\t', 1))
        self.reject(self.original.replace('\tmain\t', '\tmain\textra\t', 1))

    def test_malformed_unselected_catalog_rows_rejected(self):
        self.reject(self.original + 'bad\trow\n', ('ubuntu:noble',))

    def test_installer_rejects_before_any_apt_operation(self):
        bindir = self.root / 'bin'
        bindir.mkdir()
        marker = self.root / 'apt-ran'
        for name, body in (('id', 'echo 0'), ('apt-get', 'touch "' + str(marker) + '"; exit 99')):
            path = bindir / name
            path.write_text('#!/bin/sh\n' + body + '\n')
            path.chmod(0o755)
        self.catalog.write_text('\n'.join(line for line in self.original.splitlines() if '\tbookworm\t' not in line) + '\n')
        result = subprocess.run(['sh', str(self.root / INPUTS[0]), str(self.catalog),
                                 'ubuntu:noble', 'debian:bookworm', '--', 'curl'],
                                env=dict(os.environ, PATH=str(bindir) + os.pathsep + os.environ['PATH']),
                                capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('requested root is missing', result.stderr)
        self.assertFalse(marker.exists())


class DockerPolicy(unittest.TestCase):
    def setUp(self):
        Catalog.setUp(self)
        for name in ('scripts/install-release-dependencies.sh', '.github/scripts/install-linux-build-deps.sh'):
            target = self.root / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / name, target)
        self.dockerfile = self.root / 'Dockerfile'
        self.valid = 'FROM debian:bookworm AS runtime\nCOPY ' + ' '.join(INPUTS) + ' /opt/nyxid-apt/\nRUN ' + INSTALLER + ' debian:bookworm -- curl\n'
        self.dockerfile.write_text(self.valid)

    def test_valid_caller_and_every_actual_apt_prefix(self):
        self.assertEqual(check_apt_inputs(self.root, ['Dockerfile']), [])
        cases = images.prefixes(ROOT)
        self.assertGreaterEqual(len(cases), 12)
        for name, stage, text in cases:
            with self.subTest(name=name, stage=stage):
                self.assertIn(INSTALLER, text)
                self.assertNotIn('cargo auditable build', text)
                self.assertNotIn('from=cma', text)

    def test_minimal_ubuntu_fixture_requires_absent_system_ca_before_and_after(self):
        text = images.minimal_ubuntu(ROOT)
        self.assertEqual(text.count('test ! -s /etc/ssl/certs/ca-certificates.crt'), 2)
        self.assertIn('ubuntu:24.04@sha256:', text)
        self.assertIn('security/apt-transport-ca.pem', text)
        self.assertIn('chmod 0700 /opt/nyxid-apt', text)
        self.assertIn(INSTALLER + ' ubuntu:noble -- libdbus-1-dev', text)

    def test_raw_apt_variants_rejected(self):
        for command in ('apt-get install curl', 'apt -y install curl', '/usr/bin/apt-get -o Foo=bar update'):
            self.dockerfile.write_text(self.valid + 'RUN ' + command + '\n')
            with self.subTest(command=command):
                self.assertTrue(any('raw APT' in error for error in check_apt_inputs(self.root, ['Dockerfile'])))

    def test_missing_stage_inputs_and_unknown_selector_rejected(self):
        for text in (*[self.valid.replace(name, '') for name in INPUTS[3:]], self.valid.replace(INPUTS[1], ''), self.valid.replace('debian:bookworm --', 'debian:sid --'),
                     self.valid.replace('sh /opt/nyxid-apt/apt-snapshot-install.sh ',
                                        'sh /opt/nyxid-apt/apt-snapshot-install.sh --install-only ')):
            self.dockerfile.write_text(text)
            self.assertTrue(check_apt_inputs(self.root, ['Dockerfile']))

    def test_runner_mutable_apt_rejected(self):
        target = self.root / '.github/scripts/install-linux-build-deps.sh'
        target.write_text(target.read_text() + '\napt-get install curl\n')
        self.assertTrue(any('Linux dependencies' in error for error in check_apt_inputs(self.root, ['Dockerfile'])))

    def test_inherited_projection_requires_full_parent(self):
        parsed = stages('FROM debian:bookworm AS parent\nRUN touch /inherited\nFROM parent AS child\nRUN ' + INSTALLER + ' debian:bookworm -- curl\n')
        with self.assertRaisesRegex(ValueError, 'full inherited stage'):
            projection(parsed[1], parsed)
        parsed = stages('FROM debian:bookworm AS parent\nFROM parent AS child\nRUN ' + INSTALLER + ' debian:bookworm -- curl\n')
        self.assertIn('FROM debian:bookworm AS child', projection(parsed[1], parsed))


class Harness(unittest.TestCase):
    def test_bad_signature_changes_cleartext_only(self):
        data = b'-----BEGIN PGP SIGNED MESSAGE-----\nHash: SHA256\n\nOrigin: Debian\n-----BEGIN PGP SIGNATURE-----\n\n' + b'B' * 64 + b'\n-----END PGP SIGNATURE-----\n'
        changed = harness.mutate_signature(data, 'bad-signature')
        self.assertNotEqual(changed, data)
        self.assertEqual(changed.split(b'-----BEGIN PGP SIGNATURE-----')[1], data.split(b'-----BEGIN PGP SIGNATURE-----')[1])
        self.assertIn(b'Origin: NyxID fixture Debian', changed)

    def test_harness_rejects_early_exit_and_unreached_injection(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            for state in ({'requests': [], 'errors': [], 'injected': 0},
                          {'requests': ['/InRelease'], 'errors': ['network failed'], 'injected': 1},
                          {'requests': ['/InRelease'], 'errors': [], 'injected': 0}):
                with self.subTest(state=state), self.assertRaises(ValueError):
                    harness.validate_case('bad-index', state, 'Hash Sum mismatch', output)

    def test_harness_rejects_package_download_before_metadata_failure(self):
        with self.assertRaisesRegex(ValueError, 'preceded metadata'):
            harness.validate_case('wrong-digest', {'requests': ['/x.deb'], 'errors': [], 'injected': 0},
                                  'signed snapshot bytes differ from the reviewed catalog', ROOT)

    def test_harness_requires_crypto_diagnostic_not_catalog_rejection(self):
        state = {'requests': ['/InRelease'], 'errors': [], 'injected': 1}
        for diagnostic in ('random failure', 'BADSIG\nsigned snapshot bytes differ from the reviewed catalog'):
            with self.subTest(diagnostic=diagnostic), self.assertRaises(ValueError):
                harness.validate_case('bad-signature', state, diagnostic, ROOT)
        harness.validate_case('bad-signature', state, 'BADSIG', ROOT)


if __name__ == '__main__':
    unittest.main()
