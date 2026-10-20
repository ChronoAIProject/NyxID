#!/usr/bin/env python3
"""Portable parser and Dockerfile admission regressions (no Docker required)."""
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

from apk_inputs import ARCHES, catalog, check_apk_inputs, prefixes

ROOT = Path(__file__).resolve().parents[1]
BASE = 'alpine:3.22@sha256:' + 'a' * 64
UPSTREAM = 'https://dl-cdn.alpinelinux.org/alpine/v3.24/main/x86_64/python3-3.14.8-r0.apk'
DURABLE = 'https://github.com/ChronoAIProject/NyxID/releases/download/build-inputs-apk-2026-10-20/' + 'b' * 64 + '-x86_64-python3-3.14.8-r0.apk'
ROW = 'python3\t3.14.8-r0\tx86_64\t' + 'b' * 64 + '\t' + DURABLE + '\t' + UPSTREAM + '\n'
VALID = f'''# nyxid.apk-inputs.v2
# base-image {BASE}
# arch x86_64
# refreshed-on 2026-10-20
# packages python3
# before-sha256 {'c' * 64}
# after-sha256 {'d' * 64}
''' + ROW


class CatalogTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.path = Path(self.tmp.name) / 'catalog.tsv'

    def parse(self, value):
        self.path.write_text(value)
        return catalog(ROOT, self.path, BASE, 'x86_64')

    def reject(self, value):
        with self.assertRaises(ValueError):
            self.parse(value)

    def test_valid(self): self.assertEqual(self.parse(VALID), ROW)
    def test_wrong_schema(self): self.reject(VALID.replace('.v2', '.v1'))
    def test_missing_header(self): self.reject(VALID.replace('# arch x86_64\n', ''))
    def test_duplicate_header(self): self.reject(VALID.replace(ROW, '# arch x86_64\n' + ROW))
    def test_unknown_header(self): self.reject(VALID.replace(ROW, '# bypass yes\n' + ROW))
    def test_late_header(self): self.reject(VALID + '# arch x86_64\n')
    def test_foreign_base(self): self.reject(VALID.replace('alpine:3.22', 'alpine:3.23'))
    def test_mutable_base(self): self.reject(VALID.replace(BASE, 'alpine:3.22'))
    def test_wrong_arch(self): self.reject(VALID.replace('# arch x86_64', '# arch aarch64'))
    def test_wrong_archive_arch(self): self.reject(VALID.replace('\tx86_64\t', '\taarch64\t'))
    def test_noarch(self): self.assertIn('\tnoarch\t', self.parse(VALID.replace('\tx86_64\t', '\tnoarch\t').replace('-x86_64-python3-', '-noarch-python3-')))
    def test_bad_inventory_hash(self): self.reject(VALID.replace('c' * 64, 'c' * 63))
    def test_bad_archive_hash(self): self.reject(VALID.replace('b' * 64, 'B' * 64))
    def test_duplicate_archive(self): self.reject(VALID + ROW)
    def test_empty_closure(self): self.reject(VALID.replace(ROW, ''))
    def test_durable_date(self): self.reject(VALID.replace('build-inputs-apk-2026-10-20', 'build-inputs-apk-2026-10-21'))
    def test_durable_hash(self): self.reject(VALID.replace(DURABLE, DURABLE.replace('b' * 64, 'e' * 64)))
    def test_http_origin(self): self.reject(VALID.replace('https://', 'http://'))
    def test_origin_lookalike(self): self.reject(VALID.replace('dl-cdn.alpinelinux.org', 'dl-cdnXalpinelinuxYorg'))
    def test_origin_suffix(self): self.reject(VALID.replace('alpinelinux.org/', 'alpinelinux.org.evil/'))
    def test_moving_branch(self): self.reject(VALID.replace('/v3.24/', '/edge/'))
    def test_url_arch(self): self.reject(VALID.replace('/x86_64/', '/aarch64/'))
    def test_filename(self): self.reject(VALID.replace('/python3-3.14.8-r0.apk', '/python3-3.14.9-r0.apk'))
    def test_path_traversal(self): self.reject(VALID.replace('/main/x86_64/', '/main/../main/x86_64/'))
    def test_query(self): self.reject(VALID.replace('.apk\n', '.apk?x=1\n'))
    def test_extra_column(self): self.reject(VALID.replace('.apk\n', '.apk\textra\n'))
    def test_blank_row(self): self.reject(VALID + '\n')
    def test_option_package(self): self.reject(VALID.replace('# packages python3', '# packages --allow-untrusted'))
    def test_duplicate_request(self): self.reject(VALID.replace('# packages python3', '# packages python3 python3'))


class Callers(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.names = subprocess.check_output(['git', 'ls-files', '*Dockerfile*'], cwd=ROOT, text=True).splitlines()
        for name in self.names + ['scripts/apk-catalog.awk']:
            target = self.root / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / name, target)
        shutil.copytree(ROOT / 'security/apk', self.root / 'security/apk')

    def mutate(self, before, after, expected, name='frontend/Dockerfile'):
        path = self.root / name
        text = path.read_text()
        self.assertIn(before, text)
        path.write_text(text.replace(before, after, 1))
        self.assertTrue(any(expected in e for e in check_apk_inputs(self.root, self.names)))

    def test_current(self): self.assertEqual(check_apk_inputs(self.root, self.names), [])
    def test_raw_apk(self): self.mutate('RUN --network=none ', 'RUN apk add python3 && ', 'raw APK')
    def test_networked_install(self): self.mutate('RUN --network=none ', 'RUN ', 'network=none')
    def test_catalog_copy(self): self.mutate('COPY security/apk/', 'COPY wrong/', 'omits installer')
    def test_wrong_stage_catalog(self): self.mutate('AS runtime', 'AS wrong', 'stage/request binding')
    def test_no_download(self): self.mutate('--download ', '--install ', 'stage/request binding')
    def test_missing_arch(self):
        (self.root / 'security/apk/frontend-runtime-arm64.tsv').unlink()
        self.assertTrue(check_apk_inputs(self.root, self.names))
    def test_requested_packages(self):
        path = self.root / 'security/apk/frontend-runtime-amd64.tsv'
        path.write_text(path.read_text().replace('# packages python3', '# packages perl'))
        self.assertTrue(any('requested packages differ' in e for e in check_apk_inputs(self.root, self.names)))
    def test_override_durable_default(self): self.mutate('ARG TARGETARCH', 'ARG TARGETARCH\nARG APK_INPUT_SOURCE=provision-upstream', 'default durable-first')
    def test_writable_mount(self): self.mutate('target=/opt/nyxid-apk/archives sh', 'target=/opt/nyxid-apk/archives,rw sh', 'read-only')
    def test_copy_download_archives(self):
        path = self.root / 'frontend/Dockerfile'
        path.write_text(path.read_text() + '\nCOPY --from=frontend-runtime-apk-download /opt/nyxid-apk/archives /archives\n')
        self.assertTrue(any('must not be inherited or copied' in e for e in check_apk_inputs(self.root, self.names)))
    def test_same_stage_download_install(self):
        path = self.root / 'frontend/Dockerfile'
        text = path.read_text()
        download = next(line for line in text.splitlines() if ' --download ' in line)
        path.write_text(text.replace('RUN --network=none ', download + '\nRUN --network=none ', 1))
        self.assertTrue(any('separate stages' in e for e in check_apk_inputs(self.root, self.names)))
    def test_foreign_durable_repository(self):
        path = self.root / 'security/apk/frontend-runtime-amd64.tsv'
        path.write_text(path.read_text().replace('/ChronoAIProject/NyxID/', '/evil/NyxID/'))
        self.assertTrue(check_apk_inputs(self.root, self.names))
    def test_projection(self):
        cases = prefixes(self.root, self.names)
        self.assertEqual(len(cases), 3)
        for _, _, text in cases:
            self.assertIn('RUN --network=none ', text)
            self.assertNotIn('COPY --from=', text)
            self.assertNotIn('cargo auditable build', text)
        self.assertEqual(set(ARCHES), {'amd64', 'arm64'})


if __name__ == '__main__':
    unittest.main()
