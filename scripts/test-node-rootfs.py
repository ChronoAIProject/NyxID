#!/usr/bin/env python3
"""Exercise selected-package export with real files and a hostile builder root."""
import importlib.util
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('node_rootfs', Path(__file__).with_name('export-node-rootfs.py'))
rootfs = importlib.util.module_from_spec(spec)
spec.loader.exec_module(rootfs)


class NodeRootfs(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(); self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name); self.source = self.root / 'source'; self.dest = self.root / 'dest'
        self.files = {p: ['/usr/share/' + p + '/fixture'] for p in rootfs.PACKAGES}
        for names in self.files.values():
            for name in names:
                path = self.source / name.lstrip('/'); path.parent.mkdir(parents=True, exist_ok=True); path.write_text(name)
        self.files['libc6'] += ['/lib/aarch64-linux-gnu/libc.so.6']
        p = self.source / 'lib/aarch64-linux-gnu/libc.so.6'; p.parent.mkdir(parents=True); p.write_bytes(b'ELF fixture')
        p = self.source / 'etc/ssl/certs/ca-certificates.crt'; p.parent.mkdir(parents=True); p.write_text('public test CA')
        p = self.source / 'usr/share/nyxid/build-inputs/apt/fixture/installed-packages.tsv'; p.parent.mkdir(parents=True); p.write_text('builder inventory')
        for name in ('bin/sh', 'usr/bin/ssh', 'usr/bin/perl', 'usr/lib/libssl.so.3', 'var/lib/dpkg/info/unrelated.list'):
            p = self.source / name; p.parent.mkdir(parents=True, exist_ok=True); p.write_text('must not ship')

    def query(self, command, **kwargs):
        package = command[-1]
        if command[1] == '-L': return '\n'.join(self.files[package]) + '\n'
        return f'Package: {package}\nStatus: install ok installed\nVersion: 1.0\nArchitecture: arm64\nMulti-Arch: same\n'

    def run_export(self): rootfs.export(self.dest, self.source, self.query)

    def test_export_retains_exact_packages_ca_and_scannable_status_without_builder_tools(self):
        self.run_export()
        status = (self.dest / 'var/lib/dpkg/status').read_text()
        self.assertEqual(status.count('Package: '), 5)
        for package in rootfs.PACKAGES: self.assertIn('Package: ' + package + '\n', status)
        for name in ('bin/sh', 'usr/bin/ssh', 'usr/bin/perl', 'usr/lib/libssl.so.3', 'var/lib/dpkg/info/unrelated.list'):
            self.assertFalse((self.dest / name).exists(), name)
        self.assertEqual((self.dest / 'lib/aarch64-linux-gnu/libc.so.6').read_bytes(), b'ELF fixture')
        self.assertEqual((self.dest / 'etc/ssl/certs/ca-certificates.crt').read_text(), 'public test CA')
        self.assertTrue((self.dest / 'var/lib/dpkg/info/libc6:arm64.list').is_file())

    def test_slim_excluded_documentation_is_recorded_but_runtime_files_must_exist(self):
        self.files['base-files'].append('/usr/share/doc/base-files/README')
        self.run_export()
        import json
        record = json.loads((self.dest / 'usr/share/nyxid/build-inputs/node-rootfs.json').read_text())
        self.assertEqual(record['packages'][0]['omitted_metadata'], ['/usr/share/doc/base-files/README'])
        self.assertNotIn('/usr/share/doc/base-files/README', (self.dest / 'var/lib/dpkg/info/base-files:arm64.list').read_text())

    def test_existing_root_missing_package_file_and_missing_ca_fail(self):
        for mode in ('existing', 'missing-file', 'missing-ca'):
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as temporary:
                self.dest = Path(temporary) / 'dest'
                if mode == 'existing': self.dest.mkdir()
                elif mode == 'missing-file': self.files['netbase'].append('/missing')
                else: (self.source / 'etc/ssl/certs/ca-certificates.crt').unlink()
                with self.assertRaises((ValueError, FileNotFoundError)): self.run_export()
                if mode == 'missing-file': self.files['netbase'].pop()

    def test_uninstalled_or_wrong_package_is_rejected(self):
        for old, new in (('install ok installed', 'deinstall ok config-files'), ('Package: libc6', 'Package: other')):
            def query(command, **kwargs): return self.query(command, **kwargs).replace(old, new)
            with self.subTest(old=old), self.assertRaises(ValueError): rootfs.export(self.dest, self.source, query)
            self.assertFalse(self.dest.exists())

    def test_traversal_and_symlink_escape_are_rejected(self):
        for name in ('/../../escape', '/usr/share/escape/file'):
            self.files['netbase'].append(name)
            p = self.source / 'usr/share/escape'; p.symlink_to('/tmp') if not p.exists() else None
            with self.subTest(name=name), self.assertRaises(ValueError): self.run_export()
            self.files['netbase'].pop()
            import shutil
            if self.dest.exists(): shutil.rmtree(self.dest)


if __name__ == '__main__': unittest.main()
