#!/usr/bin/env python3
import hashlib
import json
from pathlib import Path
import struct
import tempfile
import unittest
from unittest.mock import patch

from source_artifacts import IMAGES, static_musl, verify_binary, verify_frontend, verify_labels

COMMIT, EPOCH = 'a' * 40, 1792365742


class ImageIdentity(unittest.TestCase):
    def info(self):
        return {'Os': 'linux', 'Architecture': 'amd64', 'Config': {'Labels': {
            'org.opencontainers.image.revision': COMMIT, 'io.nyxid.source-date-epoch': str(EPOCH)}}}

    def test_image_identity_matches_independent_plan(self):
        verify_labels(self.info(), 'amd64', COMMIT, EPOCH)

    def test_wrong_image_architecture_or_os(self):
        for key, value in (('Architecture', 'arm64'), ('Os', 'windows')):
            info = self.info()
            info[key] = value
            with self.subTest(key=key), self.assertRaises(ValueError): verify_labels(info, 'amd64', COMMIT, EPOCH)

    def test_missing_wrong_or_noncanonical_image_labels(self):
        for key in ('org.opencontainers.image.revision', 'io.nyxid.source-date-epoch'):
            for value in (None, '', 'unknown', '0' + str(EPOCH)):
                info = self.info()
                info['Config']['Labels'][key] = value
                with self.subTest(key=key, value=value), self.assertRaises(ValueError): verify_labels(info, 'amd64', COMMIT, EPOCH)

    def test_all_publication_components_and_auxiliary_images_covered(self):
        root = Path(__file__).resolve().parents[1]
        policy = json.loads((root / 'security/publication-policy.json').read_text())
        for component, dockerfile in policy['images']['components'].items():
            self.assertEqual(IMAGES[component][0], dockerfile)
        for component in ('machine-node-only', 'machine-hosted-desktop', 'machine-hosted', 'mcp-demo'):
            self.assertIn(component, IMAGES)


class BinaryIdentity(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.binary = Path(temporary.name) / 'nyxid'
        self.binary.write_bytes(b'\xcf\xfa\xed\xfe' + struct.pack('<III', 0x0100000C, 0, 2) + b'\0' * 48)
        self.identity = dict(schema='nyxid.build-identity.v1', package='nyxid-cli', package_version='0.72.0',
                             target='aarch64-apple-darwin', commit=COMMIT, source_date_epoch=EPOCH, source_state='exported')

    def receipt(self):
        return dict(build_identity=self.identity, subject=dict(name='nyxid', size=self.binary.stat().st_size,
                    sha256=hashlib.sha256(self.binary.read_bytes()).hexdigest()))

    def check(self, receipt):
        with patch('source_artifacts.subprocess.check_output', return_value=json.dumps(receipt).encode()):
            return verify_binary(self.binary, Path('/reviewed/extractor'), 'nyxid-cli', '0.72.0',
                                 'aarch64-apple-darwin', COMMIT, EPOCH)

    def test_final_binary_identity_and_subject(self):
        self.check(self.receipt())

    def test_stale_commit_epoch_package_version_target_and_dirty_state(self):
        for key, value in (('commit', 'b' * 40), ('source_date_epoch', EPOCH + 1), ('package', 'nyxid'),
                           ('package_version', '0.71.0'), ('target', 'x86_64-apple-darwin'),
                           ('source_state', 'git-dirty'), ('source_state', 'git-unverified'), ('source_state', 'unknown')):
            receipt = self.receipt()
            receipt['build_identity'] = {**self.identity, key: value}
            with self.subTest(key=key, value=value), self.assertRaisesRegex(ValueError, 'actual retained'): self.check(receipt)

    def test_missing_identity_and_altered_subject(self):
        receipt = self.receipt()
        receipt['build_identity'] = None
        with self.assertRaises(ValueError): self.check(receipt)
        for key, value in (('sha256', '0' * 64), ('size', 1), ('name', 'other')):
            receipt = self.receipt()
            receipt['subject'][key] = value
            with self.subTest(key=key), self.assertRaisesRegex(ValueError, 'extractor subject'): self.check(receipt)

    def test_actual_architecture_cannot_be_overridden_by_marker(self):
        self.binary.write_bytes(b'\xcf\xfa\xed\xfe' + struct.pack('<III', 0x01000007, 0, 2) + b'\0' * 48)
        with self.assertRaisesRegex(ValueError, 'architecture/format'): self.check(self.receipt())

    def test_both_linux_cli_architectures_reject_dbus_in_the_actual_packaged_elf(self):
        import importlib.util
        spec = importlib.util.spec_from_file_location('elf_fixtures', Path(__file__).with_name('test-elf-runtime.py'))
        fixtures = importlib.util.module_from_spec(spec); spec.loader.exec_module(fixtures)
        elf = fixtures.elf
        for arch, machine in (('x86_64', 62), ('aarch64', 183)):
            target = arch + '-unknown-linux-gnu'
            for libraries in (['libc.so.6'], ['libc.so.6', 'libdbus-1.so.3']):
                interpreter = {'x86_64': '/lib64/ld-linux-x86-64.so.2', 'aarch64': '/lib/ld-linux-aarch64.so.1'}[arch]
                data = elf(libraries, interpreter)
                struct.pack_into('<H', data, 18, machine)
                struct.pack_into('<Q', data, 24, 0x10100)
                self.binary.write_bytes(data)
                self.identity['target'] = target
                with self.subTest(target=target, libraries=libraries), patch('source_artifacts.subprocess.check_output',
                        return_value=json.dumps(self.receipt()).encode()):
                    if any('libdbus' in name for name in libraries):
                        with self.assertRaisesRegex(ValueError, 'forbidden libdbus'):
                            verify_binary(self.binary, Path('/extractor'), 'nyxid-cli', '0.72.0', target, COMMIT, EPOCH)
                    else:
                        verify_binary(self.binary, Path('/extractor'), 'nyxid-cli', '0.72.0', target, COMMIT, EPOCH)

    def test_scratch_updater_requires_static_musl(self):
        header = bytearray(64)
        header[:6] = b'\x7fELF\x02\x01'
        struct.pack_into('<HH', header, 16, 3, 183)
        struct.pack_into('<Q', header, 32, 64)
        struct.pack_into('<HH', header, 54, 56, 1)
        self.binary.write_bytes(header + struct.pack('<I', 1) + b'\0' * 52)
        static_musl(self.binary, 'aarch64-unknown-linux-musl')
        self.binary.write_bytes(header + struct.pack('<I', 3) + b'\0' * 52)
        with self.assertRaisesRegex(ValueError, 'without an interpreter'): static_musl(self.binary, 'aarch64-unknown-linux-musl')
        with self.assertRaises(ValueError): static_musl(self.binary, 'x86_64-unknown-linux-musl')


class FrontendIdentity(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.dist, self.source = self.root / 'dist', self.root / 'source'
        (self.dist / 'assets').mkdir(parents=True)
        (self.source / 'frontend').mkdir(parents=True)
        (self.source / 'frontend/package-lock.json').write_text('{}')
        self.write('build-identity.json', dict(schema='nyxid.frontend-build-identity.v1', commit=COMMIT,
                   source_date_epoch=EPOCH, source_state='exported'))
        self.write('build-version.json', dict(commit=COMMIT, buildId=COMMIT, assets=['assets/main.js']))
        (self.dist / 'assets/main.js').write_text('app()')
        (self.dist / 'index.html').write_text('<script src="/assets/main.js"></script>')
        self.seal()

    def write(self, name, value):
        (self.dist / name).write_text(json.dumps(value))

    def seal(self):
        files = [path for path in self.dist.rglob('*') if path.is_file() and path.name != 'shipped-composition.json']
        self.write('shipped-composition.json', dict(schema='nyxid.shipped-composition.v1', component='spa',
            source_lock_sha256=hashlib.sha256((self.source / 'frontend/package-lock.json').read_bytes()).hexdigest(),
            artifacts=[dict(path=path.relative_to(self.dist).as_posix(), size=path.stat().st_size,
                            sha256=hashlib.sha256(path.read_bytes()).hexdigest()) for path in files]))

    def check(self):
        return verify_frontend(self.dist, self.source, COMMIT, EPOCH)

    def test_full_frontend_identity_and_sealed_bytes(self):
        self.assertEqual(self.check()['commit'], COMMIT)

    def test_missing_frontend_identity(self):
        (self.dist / 'build-identity.json').unlink()
        with self.assertRaises(ValueError): self.check()

    def test_wrong_commit_epoch_or_unverified_source_even_when_resealed(self):
        original = json.loads((self.dist / 'build-identity.json').read_text())
        for key, value in (('commit', 'b' * 40), ('source_date_epoch', EPOCH + 1), ('source_state', 'unverified')):
            self.write('build-identity.json', {**original, key: value})
            self.seal()
            with self.subTest(key=key), self.assertRaisesRegex(ValueError, 'frontend identity'): self.check()

    def test_update_metadata_cannot_name_another_commit(self):
        self.write('build-version.json', dict(commit='b' * 40, buildId=COMMIT, assets=['assets/main.js']))
        with self.assertRaisesRegex(ValueError, 'update metadata'): self.check()

    def test_initial_asset_set_must_exist(self):
        self.write('build-version.json', dict(commit=COMMIT, buildId=COMMIT, assets=['assets/missing.js']))
        with self.assertRaisesRegex(ValueError, 'initial assets'): self.check()

    def test_changed_missing_or_extra_final_assets(self):
        asset = self.dist / 'assets/main.js'
        asset.write_text('changed()')
        with self.assertRaisesRegex(ValueError, 'composition bytes'): self.check()
        (self.dist / 'extra.js').write_text('extra()')
        with self.assertRaisesRegex(ValueError, 'closure mismatch'): self.check()
        asset.unlink()
        with self.assertRaisesRegex(ValueError, 'initial assets'): self.check()

    def test_source_lock_and_symlink_rejected(self):
        (self.source / 'frontend/package-lock.json').write_text('{"changed":true}')
        with self.assertRaisesRegex(ValueError, 'source lock'): self.check()
        (self.dist / 'extra').symlink_to(self.source / 'frontend/package-lock.json')
        with self.assertRaisesRegex(ValueError, 'nonregular'): self.check()


if __name__ == '__main__':
    unittest.main()
