"""Exercise the updater staging guard with compiler-style dep-info and omissions."""
import contextlib
import importlib.util
import io
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

SCRIPT = Path(__file__).with_name('check-updater-docker-embeds.py')
SPEC = importlib.util.spec_from_file_location('updater_embeds', SCRIPT)
embeds = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(embeds)


class UpdaterEmbedsTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.production = self.root / 'production.d'
        self.tests = self.root / 'tests.d'
        self.production.write_text(
            'out: /build/cli/src/tls.rs /build/cli/resources/sigstore-root.json '
            '/build/cli/Cargo.toml /build/machine/resources/cua-tools.json '
            '/build/machine-updater/src/main.rs\n\n/build/cli/src/tls.rs:\n'
        )
        self.tests.write_text(
            'out: cli/src/tls.rs cli/src/tls/tests.rs '
            'cli/tests/fixtures/sigstore/trusted_root.json\n'
        )
        self.dockerfile = self.root / 'Dockerfile'
        self.original = embeds.DEFAULT_DOCKERFILE.read_text()
        self.dockerfile.write_text(self.original)

    def check(self):
        output = io.StringIO()
        with contextlib.redirect_stderr(output):
            result = embeds.check([self.production], [self.tests], self.dockerfile,
                                  embeds.DEFAULT_DOCKERIGNORE)
        return result, output.getvalue()

    def test_production_and_test_closures_use_their_own_stages(self):
        (status, production, tests), errors = self.check()
        self.assertEqual(status, 0, errors)
        self.assertIn('machine/resources/cua-tools.json', production)
        self.assertEqual(tests, {'cli/src/tls/tests.rs',
                                'cli/tests/fixtures/sigstore/trusted_root.json'})

    def test_staging_and_cache_regressions_are_detected(self):
        for old, new, message in [
            ('COPY cli/resources/sigstore-root.json cli/resources/sigstore-root.json\n', '', 'sigstore-root.json'),
            ('FROM test-dependencies AS tests-build', 'FROM builder AS tests-build', 'sibling'),
            ('RUN cargo build --locked', 'RUN --mount=type=cache,target=/build/target cargo build --locked', 'cache mounts'),
            ('COPY machine-updater/src machine-updater/src\n', 'COPY machine-updater/ machine-updater/\n', 'broad workspace copy'),
            ('COPY machine/Cargo.toml machine/Cargo.toml\n', '', 'machine/Cargo.toml'),
            ('FROM dependencies AS builder', 'FROM dependencies AS builder\nCOPY cli/tests/fixtures cli/tests/fixtures', 'fixture is in the production stage'),
        ]:
            with self.subTest(message=message):
                self.dockerfile.write_text(self.original.replace(old, new, 1))
                (status, _, _), errors = self.check()
                self.assertEqual(status, 1)
                self.assertIn(message, errors)

    def test_empty_production_closure_fails(self):
        self.production.write_text('out: /registry/dependency.rs\n')
        (status, _, _), errors = self.check()
        self.assertEqual(status, 1)
        self.assertIn('no repository inputs', errors)

    def test_empty_test_evidence_fails(self):
        self.tests.write_text('out: /registry/dependency.rs\n')
        (status, _, _), errors = self.check()
        self.assertEqual(status, 1)
        self.assertIn('test dep-info contains no repository inputs', errors)
        empty = self.root / 'empty'
        empty.mkdir()
        with patch.object(embeds.sys, 'argv', [str(SCRIPT), '--dep-info', str(self.production),
                                             '--test-dep-info', str(empty)]), \
                contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(embeds.main(), 1)

    def test_dep_info_paths_and_phony_rules(self):
        self.production.write_text('out: /build/cli/src/tls.rs \\\n /build/path\\ with\\ spaces.rs\n\n/build/phony.rs:\n')
        self.assertEqual(embeds.parse_dep_info(self.production), [
            embeds.REPO_ROOT / 'cli/src/tls.rs', embeds.REPO_ROOT / 'path with spaces.rs',
        ])

    def test_parent_stages_are_followed(self):
        parsed = embeds.stages('FROM scratch AS sources\nCOPY src/ src/\nFROM sources AS builder\n')
        self.assertEqual(embeds.reachable_copies(parsed, 'builder'), ['src/'])

    def test_dockerignore_inclusions(self):
        patterns = [(False, 'docs/'), (True, 'docs/kept.md'), (False, '*.pem')]
        self.assertTrue(embeds.is_ignored(Path('docs/dropped.md'), patterns))
        self.assertFalse(embeds.is_ignored(Path('docs/kept.md'), patterns))
        self.assertTrue(embeds.is_ignored(Path('keys/private.pem'), patterns))


if __name__ == '__main__':
    unittest.main()
