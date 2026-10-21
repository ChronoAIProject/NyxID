#!/usr/bin/env python3
import base64
from datetime import datetime, timezone
import hashlib
import io
import json
from pathlib import Path
import struct
import tarfile
import tempfile
import unittest

from credential_packaging import GRAPH, HTML, MANIFEST, SEAL, parity, verify
from image_artifacts import unpack

COMMIT, EPOCH = 'a' * 40, 1792365742
SCRIPT = 'credential-accept/assets/credential-accept-test.js'


class CredentialPackaging(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.dist, self.source = self.root / 'dist', self.root / 'source'
        (self.dist / 'credential-accept/assets').mkdir(parents=True)
        (self.dist / 'release-integrity').mkdir()
        (self.source / 'frontend').mkdir(parents=True)
        self.write_source('package.json', {'version': '0.72.0'})
        self.write_source('package-lock.json', {'lockfileVersion': 3})
        data = b'globalThis.credentialPage = true;'
        (self.dist / SCRIPT).write_bytes(data)
        sri = 'sha384-' + base64.b64encode(hashlib.sha384(data).digest()).decode()
        (self.dist / HTML).write_text('<html><script type="module" src="/' + SCRIPT + '" integrity="' + sri +
                                     '" crossorigin="anonymous" data-nyx-integrity-role="credential_accept_script"></script></html>')
        encoded = ('/' + SCRIPT).encode()
        fingerprint = hashlib.sha384(b'nyxid:rci-accept:v1\0' + struct.pack('>I', len(encoded)) + encoded +
                                     struct.pack('>Q', len(data)) + data).hexdigest()
        artifacts = []
        for name, role, content_type in ((HTML, 'credential_accept_html', 'text/html; charset=utf-8'),
                                         (SCRIPT, 'credential_accept_script', 'text/javascript; charset=utf-8')):
            content = (self.dist / name).read_bytes()
            digest = hashlib.sha384(content).digest()
            artifacts.append(dict(role=role, path='/' + name, content_type=content_type, size_bytes=len(content),
                                  sha384_hex=digest.hex(), sha384_sri='sha384-' + base64.b64encode(digest).decode()))
        self.write(MANIFEST, dict(schema_version='nyxid.release-integrity.v1', app_version='0.72.0', git_commit=COMMIT,
            generated_at=datetime.fromtimestamp(EPOCH, timezone.utc).isoformat(timespec='milliseconds').replace('+00:00', 'Z'),
            credential_accept={'fingerprint_sha384_hex': fingerprint}, artifacts=artifacts))
        lock = hashlib.sha256((self.source / 'frontend/package-lock.json').read_bytes()).hexdigest()
        self.write(GRAPH, dict(schema='nyxid.rendered-composition.v1', component='credential-accept', lock_sha256=lock))
        self.write(SEAL, dict(schema='nyxid.shipped-composition.v1', component='credential-accept', source_lock_sha256=lock,
            graphs=[dict(path=GRAPH, sha256=hashlib.sha256((self.dist / GRAPH).read_bytes()).hexdigest())],
            artifacts=[dict(path=name, size=(self.dist / name).stat().st_size,
                            sha256=hashlib.sha256((self.dist / name).read_bytes()).hexdigest())
                       for name in (HTML, SCRIPT, MANIFEST, GRAPH)]))

    def write_source(self, name, value):
        (self.source / 'frontend' / name).write_text(json.dumps(value))

    def write(self, name, value):
        (self.dist / name).write_text(json.dumps(value))

    def mutate(self, name, change):
        value = json.loads((self.dist / name).read_text())
        change(value)
        self.write(name, value)

    def check(self):
        return verify(self.dist, self.source, COMMIT, EPOCH)

    def test_complete_image_bytes_validate(self):
        files = self.check()
        self.assertEqual(set(files), {HTML, SCRIPT, MANIFEST, GRAPH, SEAL})
        parity(files, dict(files))

    def test_wrong_commit(self):
        self.mutate(MANIFEST, lambda value: value.update(git_commit='b' * 40))
        with self.assertRaisesRegex(ValueError, 'source/version/epoch'): self.check()

    def test_wrong_epoch(self):
        with self.assertRaisesRegex(ValueError, 'source/version/epoch'):
            verify(self.dist, self.source, COMMIT, EPOCH + 1)

    def test_wrong_version(self):
        self.write_source('package.json', {'version': '0.73.0'})
        with self.assertRaisesRegex(ValueError, 'source/version/epoch'): self.check()

    def test_changed_script(self):
        (self.dist / SCRIPT).write_bytes(b'changed')
        with self.assertRaisesRegex(ValueError, 'bytes differ'): self.check()

    def test_missing_script(self):
        (self.dist / SCRIPT).unlink()
        with self.assertRaisesRegex(ValueError, 'missing or duplicate'): self.check()

    def test_extra_script(self):
        (self.dist / 'credential-accept/assets/credential-accept-new.js').write_text('extra')
        with self.assertRaisesRegex(ValueError, 'extra packaged'): self.check()

    def test_duplicate_artifact(self):
        self.mutate(MANIFEST, lambda value: value['artifacts'].append(value['artifacts'][0]))
        with self.assertRaisesRegex(ValueError, 'duplicate'): self.check()

    def test_traversal_artifact(self):
        self.mutate(MANIFEST, lambda value: value['artifacts'][1].update(path='/credential-accept/assets/../test.js'))
        with self.assertRaisesRegex(ValueError, 'role/path'): self.check()

    def test_wrong_hash_fields_and_size(self):
        original = (self.dist / MANIFEST).read_bytes()
        for key, value in (('sha384_sri', 'sha384-AAAA'), ('sha384_hex', '0' * 96), ('size_bytes', 0),
                           ('content_type', 'text/plain'), ('size_bytes', True)):
            with self.subTest(key=key):
                (self.dist / MANIFEST).write_bytes(original)
                self.mutate(MANIFEST, lambda document: document['artifacts'][1].update({key: value}))
                with self.assertRaisesRegex(ValueError, 'bytes differ'): self.check()

    def test_wrong_fingerprint(self):
        self.mutate(MANIFEST, lambda value: value['credential_accept'].update(fingerprint_sha384_hex='0' * 96))
        with self.assertRaisesRegex(ValueError, 'fingerprint'): self.check()

    def test_html_sri_metadata_and_unlisted_script(self):
        # Update the HTML artifact hash to isolate HTML validation from hash validation.
        original = (self.dist / HTML).read_text()
        for content in (original.replace('integrity=', 'other='), original.replace('crossorigin="anonymous"', ''),
                        original.replace('data-nyx-integrity-role=', 'other='), original + '<script src="https://bad.invalid/a.js"></script>',
                        original + '<script>attack()</script>', original.replace('type="module"', 'type="text/javascript"')):
            with self.subTest(content=content):
                (self.dist / HTML).write_text(content)
                digest = hashlib.sha384(content.encode()).digest()
                self.mutate(MANIFEST, lambda value: value['artifacts'][0].update(size_bytes=len(content), sha384_hex=digest.hex(),
                            sha384_sri='sha384-' + base64.b64encode(digest).decode()))
                with self.assertRaisesRegex(ValueError, 'HTML|inline'): self.check()

    def test_symlink_asset_and_directory(self):
        (self.dist / SCRIPT).unlink()
        (self.dist / SCRIPT).symlink_to(self.source / 'frontend/package.json')
        with self.assertRaisesRegex(ValueError, 'nonregular'): self.check()
        (self.dist / SCRIPT).unlink()
        (self.dist / 'credential-accept/assets').rmdir()
        (self.dist / 'credential-accept/assets').symlink_to(self.source, target_is_directory=True)
        with self.assertRaisesRegex(ValueError, 'nonregular'): self.check()

    def test_missing_manifest_and_seal(self):
        for name in (MANIFEST, SEAL):
            with self.subTest(name=name):
                data = (self.dist / name).read_bytes()
                (self.dist / name).unlink()
                with self.assertRaises(ValueError): self.check()
                (self.dist / name).write_bytes(data)

    def test_duplicate_json_key(self):
        path = self.dist / MANIFEST
        path.write_text(path.read_text().replace('{', '{"git_commit":"bad",', 1))
        with self.assertRaisesRegex(ValueError, 'duplicate JSON'): self.check()

    def test_changed_composition_lock(self):
        self.write_source('package-lock.json', {'changed': True})
        with self.assertRaisesRegex(ValueError, 'locked source'): self.check()

    def test_changed_graph_and_seal_inventory(self):
        original = (self.dist / GRAPH).read_bytes()
        (self.dist / GRAPH).write_bytes(original + b' ')
        with self.assertRaisesRegex(ValueError, 'composition bytes'): self.check()
        (self.dist / GRAPH).write_bytes(original)
        self.mutate(SEAL, lambda value: value['artifacts'].pop())
        with self.assertRaisesRegex(ValueError, 'inventory mismatch'): self.check()

    def test_image_parity_rejects_even_valid_different_builds(self):
        first = self.check()
        with self.assertRaisesRegex(ValueError, 'bytes differ'):
            parity(first, {**first, SCRIPT: '0' * 64})


class ImageArchive(unittest.TestCase):
    def archive(self, members):
        stream = io.BytesIO()
        with tarfile.open(fileobj=stream, mode='w') as archive:
            for name, kind in members:
                info = tarfile.TarInfo(name)
                info.type, info.size, info.linkname = kind, 1 if kind == tarfile.REGTYPE else 0, '/outside'
                archive.addfile(info, io.BytesIO(b'x') if info.size else None)
        stream.seek(0)
        return stream

    def extract(self, members, limit=100):
        with tempfile.TemporaryDirectory() as directory:
            return unpack(self.archive(members), Path(directory) / 'output', limit)

    def test_regular_docker_cp_archive(self):
        self.assertEqual(self.extract([('.', tarfile.DIRTYPE), ('./assets', tarfile.DIRTYPE),
                                      ('./assets/test.js', tarfile.REGTYPE)]), ['assets/test.js'])

    def test_unsafe_paths_duplicate_and_links_fail(self):
        for name, kind in (('../outside', tarfile.REGTYPE), ('/outside', tarfile.REGTYPE),
                            ('assets//a', tarfile.REGTYPE), ('a\\b', tarfile.REGTYPE),
                            ('link', tarfile.SYMTYPE), ('hardlink', tarfile.LNKTYPE), ('fifo', tarfile.FIFOTYPE)):
            with self.subTest(name=name), self.assertRaises(ValueError): self.extract([(name, kind)])
        with self.assertRaisesRegex(ValueError, 'duplicate'):
            self.extract([('a', tarfile.REGTYPE), ('a', tarfile.REGTYPE)])

    def test_archive_size_bound(self):
        with self.assertRaisesRegex(ValueError, 'size bound'): self.extract([('a', tarfile.REGTYPE)], limit=0)


if __name__ == '__main__':
    unittest.main()
