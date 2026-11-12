"""Compare real release-check dep-info with a full build, including cached units."""
import importlib.util
from contextlib import redirect_stderr, redirect_stdout
import io
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location('embeds', Path(__file__).with_name('check-backend-docker-embeds.py'))
embeds = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(embeds)


class DepInfoTests(unittest.TestCase):
    def test_git_failure_sentinels_are_exempt_without_hiding_other_missing_inputs(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            inputs = []
            for package in ('backend', 'cli'):
                source = root / package / 'build.rs'
                source.parent.mkdir()
                source.write_text('fn main() {}')
                inputs.extend([source, source / 'git-stamp-unavailable'])
            dep_info = root / 'binary.d'
            dockerfile = root / 'Dockerfile'
            dockerfile.write_text('FROM rust AS builder\nCOPY backend/ backend/\nCOPY cli/ cli/\n')
            dockerignore = root / '.dockerignore'
            dockerignore.write_text('')
            with patch.object(embeds, 'REPO_ROOT', root):
                for extra in (None, root / 'backend/git-stamp-unavailable',
                              root / 'backend/build.rs/other-missing-input'):
                    with self.subTest(extra=extra):
                        paths = inputs + ([extra] if extra else [])
                        dep_info.write_text('binary: ' + ' '.join(shlex.quote(str(p)) for p in paths) + '\n')
                        with redirect_stdout(io.StringIO()), redirect_stderr(io.StringIO()):
                            result = embeds.check(dep_info, dockerfile, dockerignore)
                        self.assertEqual(result, 1 if extra else 0)

    def test_empty_file_and_empty_rule_are_not_input_evidence(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'unit.d'
            for contents in ('', 'artifact: \n'):
                with self.subTest(contents=contents):
                    path.write_text(contents)
                    with self.assertRaises(ValueError):
                        embeds.parse_dep_info(path)

    def test_git_stamp_paths_resolve_relative_to_repo_with_old_git(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            repo = root / 'repo'
            repo.mkdir()
            real_git = shutil.which('git')

            def git(*args):
                return subprocess.check_output([real_git, *args], cwd=repo, stderr=subprocess.DEVNULL)

            git('init', '-q')
            git('symbolic-ref', 'HEAD', 'refs/heads/main')
            git('-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid',
                'commit', '--allow-empty', '-qm', 'first')
            shim_dir = root / 'bin'
            shim_dir.mkdir()
            shim = shim_dir / 'git'
            shim.write_text('#!/bin/sh\nfor arg do\ncase "$arg" in --path-format*) exit 129;; esac\ndone\n'
                            f'exec {shlex.quote(real_git)} "$@"\n')
            shim.chmod(0o755)
            with patch.dict(os.environ, PATH=str(shim_dir) + os.pathsep + os.environ['PATH']):
                with patch.object(embeds, 'REPO_ROOT', repo):
                    self.assertEqual(embeds.git_stamp_inputs(), {
                        (repo / '.git/HEAD').resolve(), (repo / '.git/logs/HEAD').resolve(),
                        (repo / '.git/refs/heads/main').resolve(),
                    })
                    git('pack-refs', '--all', '--prune')
                    self.assertEqual(embeds.git_stamp_inputs(), {
                        (repo / '.git/HEAD').resolve(), (repo / '.git/logs/HEAD').resolve(),
                    })
                    (repo / '.git/logs/HEAD').unlink()
                    self.assertEqual(embeds.git_stamp_inputs(), {
                        (repo / '.git/HEAD').resolve(), (repo / '.git/logs/HEAD').resolve(),
                    })
                linked = root / 'linked'
                git('worktree', 'add', '-b', 'side', str(linked))
                with patch.object(embeds, 'REPO_ROOT', linked):
                    paths = embeds.git_stamp_inputs()
                    self.assertEqual(paths, {
                        (repo / '.git/worktrees/linked/HEAD').resolve(),
                        (repo / '.git/worktrees/linked/logs/HEAD').resolve(),
                        (repo / '.git/refs/heads/side').resolve(),
                    })
                with patch.object(embeds, 'REPO_ROOT', root):
                    self.assertEqual(embeds.git_stamp_inputs(), set())


@unittest.skipUnless(shutil.which('cargo'), 'requires Cargo')
class CargoCheckInputsTests(unittest.TestCase):
    def test_exact_inputs_cached_units_and_stale_dep_info(self):
        with tempfile.TemporaryDirectory(prefix='docker inputs ') as directory:
            root = Path(directory).resolve()
            (root / 'backend/src').mkdir(parents=True)
            (root / 'leaf/src').mkdir(parents=True)
            (root / 'Cargo.toml').write_text('[workspace]\nmembers=["backend","leaf"]\nresolver="3"\n')
            (root / 'backend/Cargo.toml').write_text(
                '[package]\nname="nyxid"\nversion="0.1.0"\nedition="2024"\n'
                '[features]\ngcp-kms=[]\n[dependencies]\nleaf={path="../leaf"}\n'
                '[[bin]]\nname="nyxid-server"\npath="src/main.rs"\n')
            (root / 'leaf/Cargo.toml').write_text('[package]\nname="leaf"\nversion="0.1.0"\nedition="2024"\n')
            (root / 'leaf/src/lib.rs').write_text('pub fn value() -> usize { include_bytes!("../../asset file").len() }\n')
            (root / 'asset file').write_text('embedded')
            (root / 'watched file').write_text('build script input')
            (root / 'backend/build.rs').write_text(
                'fn main() { println!("cargo:rerun-if-changed=build.rs"); '
                'println!("cargo::rerun-if-changed=../watched file"); }\n')
            (root / 'backend/src/main.rs').write_text('mod extra; fn main() { println!("{}", leaf::value() + extra::value()); }\n')
            (root / 'backend/src/extra.rs').write_text('pub fn value() -> usize { 1 }\n')
            env = {**os.environ, 'CARGO_TARGET_DIR':str(root / 'target'), 'RUSTUP_TOOLCHAIN':'1.98.1',
                   'RUSTFLAGS':'-Dwarnings', 'CARGO_INCREMENTAL':'0'}
            env.pop('CARGO_ENCODED_RUSTFLAGS', None)

            def cargo(mode):
                result = subprocess.run(['cargo', mode, '--release', '-p', 'nyxid', '--bin', 'nyxid-server',
                                         '--features', 'gcp-kms', '--message-format=json'],
                                        cwd=root, env=env, capture_output=True, text=True, timeout=120)
                self.assertEqual(result.returncode, 0, result.stderr)
                path = root / (mode + '.jsonl')
                path.write_text(result.stdout)
                return path

            with patch.object(embeds, 'REPO_ROOT', root):
                cargo('build')
                expected = set(embeds.parse_dep_info(root / 'target/release/nyxid-server.d'))
                messages = cargo('check')
                self.assertEqual(set(embeds.cargo_check_inputs(messages)), expected)
                self.assertIn(root / 'watched file', expected)
                self.assertIn(root / 'asset file', expected)
                cached = cargo('check')
                self.assertEqual(set(embeds.cargo_check_inputs(cached)), expected)
                units = [json.loads(line) for line in cached.read_text().splitlines()]
                self.assertTrue(all(m['fresh'] for m in units if m['reason'] == 'compiler-artifact'))
                (root / 'backend/src/extra.rs').unlink()
                (root / 'backend/src/main.rs').write_text('fn main() { println!("{}", leaf::value()); }\n')
                # Keep the obsolete aggregate .d and an unrelated stale unit .d.
                (root / 'target/release/deps/stale.d').write_text('stale: missing-input\n')
                current = set(embeds.cargo_check_inputs(cargo('check')))
                self.assertEqual(current, expected - {root / 'backend/src/extra.rs'})
                self.assertIn(root / 'backend/src/extra.rs', set(embeds.parse_dep_info(root / 'target/release/nyxid-server.d')))
                # An incomplete/failed stream cannot turn a missing backend into success.
                original = messages.read_text()
                for changes in ('missing-finish', 'failed', 'wrong-feature', 'test-profile'):
                    rows = [json.loads(line) for line in original.splitlines()]
                    for row in rows:
                        if row['reason'] == 'build-finished' and changes == 'failed':
                            row['success'] = False
                        if row['reason'] == 'compiler-artifact' and row['target']['name'] == 'nyxid-server':
                            if changes == 'wrong-feature': row['features'] = []
                            if changes == 'test-profile': row['profile']['test'] = True
                    if changes == 'missing-finish': rows = [r for r in rows if r['reason'] != 'build-finished']
                    messages.write_text('\n'.join(json.dumps(row) for row in rows))
                    with self.subTest(changes=changes), self.assertRaises(ValueError):
                        embeds.cargo_check_inputs(messages)


if __name__ == '__main__':
    unittest.main()
