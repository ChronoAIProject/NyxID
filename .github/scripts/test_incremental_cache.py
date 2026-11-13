import io
import fcntl
from contextlib import redirect_stdout
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import incremental_cache as cache
import backend_disk

ROOT = Path(__file__).resolve().parents[2]


class IncrementalTests(unittest.TestCase):
    def test_real_cargo_prunes_old_version_and_preserves_fresh_graphs(self):
        with tempfile.TemporaryDirectory(prefix='incremental cache ') as tmp:
            root = Path(tmp)
            (root / 'src').mkdir()
            source = root / 'src/lib.rs'
            source.write_text('pub fn answer() -> u32 { 42 }\n#[test] fn test() { assert_eq!(answer(), 42); }')
            env = {**os.environ, 'RUSTUP_TOOLCHAIN': '1.98.1', 'CARGO_INCREMENTAL': '1',
                   'RUSTFLAGS': '-Dwarnings', 'CARGO_TERM_COLOR': 'never', 'CARGO_TARGET_DIR': str(root / 'target')}
            for name in ('CARGO_ENCODED_RUSTFLAGS', 'RUSTC_WRAPPER', 'RUSTC_WORKSPACE_WRAPPER'):
                env.pop(name, None)
            def run(*args):
                result = subprocess.run(args, cwd=root, env=env, text=True, capture_output=True, timeout=120)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                return result
            def helper(command):
                return run(sys.executable, str(ROOT / '.github/scripts/incremental_cache.py'), command)
            def build():
                return run('cargo', 'test', '--no-run')
            def version(value):
                (root / 'Cargo.toml').write_text('[package]\nname="fixture"\nversion="' + value + '"\n[workspace]\n')
            graphs = root / 'target/debug/incremental'
            version('0.1.0')
            build()
            first = {p.name for p in graphs.iterdir()}
            self.assertTrue(first)
            helper('mark')
            version('0.2.0')
            build()
            self.assertGreater(len(list(graphs.iterdir())), len(first))
            result = helper('prune')
            after = {p.name for p in graphs.iterdir()}
            self.assertEqual(len(after), len(first))
            self.assertFalse(first & after)
            print(result.stdout)
            # Fresh checkout mtimes force all selected units to finalize new
            # sessions even when source content and package version are unchanged.
            helper('mark')
            source.touch()
            self.assertIn('Compiling fixture', build().stderr)
            # A job's marker covers all its Cargo commands, including clippy.
            run('cargo', 'clippy', '--all-targets', '--', '-D', 'warnings')
            helper('prune')
            self.assertTrue(after <= {p.name for p in graphs.iterdir()})
            for crate in graphs.iterdir():
                self.assertEqual(sum(p.is_dir() for p in crate.iterdir()), 1)
            fresh = build()
            self.assertNotIn('Compiling fixture', fresh.stdout + fresh.stderr)
            helper('prune')
            # A local fresh target with a NEW marker produces no sessions.
            # CI saves only after a fresh checkout; refuse an empty save here.
            helper('mark')
            fresh = build()
            self.assertNotIn('Compiling fixture', fresh.stdout + fresh.stderr)
            failed = subprocess.run([sys.executable, str(ROOT / '.github/scripts/incremental_cache.py'), 'prune'],
                                    cwd=root, env=env, text=True, capture_output=True)
            self.assertNotEqual(failed.returncode, 0)
            self.assertIn('empty graph', failed.stderr)

    def test_session_pruning_respects_locks_and_uses_encoded_timestamp(self):
        with tempfile.TemporaryDirectory() as tmp:
            crate = Path(tmp)
            old = crate / 's-000a-aaa-aaa'
            latest = crate / 's-000b-bbb-bbb'
            working = crate / 's-000c-ccc-working'
            for entry in (old, latest, working):
                entry.mkdir()
                (entry / 'graph').write_bytes(b'graph')
                (crate / ('-'.join(entry.name.split('-')[:3]) + '.lock')).touch()
            with (crate / 's-000a-aaa.lock').open('rb') as locked:
                fcntl.flock(locked, fcntl.LOCK_EX | fcntl.LOCK_NB)
                with self.assertRaises(BlockingIOError):
                    cache.prune_sessions(crate)
                self.assertTrue(old.exists())
                self.assertTrue(latest.exists())
            cache.prune_sessions(crate)
            self.assertEqual({p.name for p in crate.iterdir()}, {latest.name, 's-000b-bbb.lock'})

    def test_missing_marker_refuses_save(self):
        with tempfile.TemporaryDirectory() as tmp, patch.object(cache, 'MARKER', Path(tmp) / 'missing'):
            with self.assertRaisesRegex(ValueError, 'no build marker'):
                cache.prune()

    def test_budget_covers_download_extraction_and_build_growth(self):
        swap = backend_disk.reserve_bytes('SwapTotal: 3145728 kB')
        self.assertEqual(swap, 37 * 1024**3 + 1024**2)
        self.assertEqual(backend_disk.reserve_bytes("SwapTotal: 3145724 kB"),
                         37 * 1024**3 + 2 * 1024**2)
        self.assertEqual(backend_disk.reserve_bytes("SwapTotal: 41943040 kB"), 0)
        self.assertEqual(backend_disk.restore_required(swap), 89 * 1024**3 + 1024**2)
        self.assertGreaterEqual(backend_disk.restore_required(0), 41 * 1024**3)

    def test_low_disk_skips_restore_and_refuses_save_before_archive(self):
        with tempfile.TemporaryDirectory() as tmp:
            output = Path(tmp) / 'output'
            for free, allowed in ((0, 'false'), (200 * backend_disk.GIB, 'true')):
                output.write_text('')
                with patch.dict(os.environ, {'GITHUB_OUTPUT': str(output)}), \
                     patch.object(sys, 'argv', ['backend_disk', 'before-incremental']), \
                     patch('backend_disk.shutil.disk_usage') as usage, \
                     patch('backend_disk.subprocess.run'), redirect_stdout(io.StringIO()):
                    usage.return_value.free = free
                    backend_disk.main()
                self.assertEqual(output.read_text(), f'allowed={allowed}\n')
            with patch.object(sys, 'argv', ['backend_disk', 'before-save']), \
                 patch('backend_disk.shutil.disk_usage') as usage, \
                 patch('backend_disk.subprocess.run'):
                usage.return_value.free = 24 * backend_disk.GIB
                with self.assertRaisesRegex(SystemExit, 'not saving'):
                    backend_disk.main()

    def test_oversize_graph_refuses_save_and_symlink_root_is_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / 'incremental'
            unit = root / 'fixture-0123456789abcdef'
            unit.mkdir(parents=True)
            (unit / 'graph').write_bytes(b'graph')
            marker = Path(tmp) / 'marker'
            marker.touch()
            with patch.object(cache, 'ROOTS', (root,)), patch.object(cache, 'MARKER', marker), \
                 patch.object(cache, 'MAX_BYTES', 1), patch('incremental_cache.prune_sessions'), \
                 patch('incremental_cache.subprocess.run'), \
                 redirect_stdout(io.StringIO()):
                with self.assertRaisesRegex(ValueError, 'exceeds'):
                    cache.prune()
                link = Path(tmp) / 'link'
                link.symlink_to(root)
                with patch.object(cache, 'ROOTS', (link,)), self.assertRaisesRegex(ValueError, 'symlink'):
                    cache.inventory('test')

    def test_bounded_size_includes_sparse_file_size_and_tar_overhead(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            with (root / 'sparse').open('wb') as f:
                f.truncate(1024**2)
            self.assertEqual(cache.size(root), 1024**2 + 8192)
            (root / 'symlink').symlink_to(root / 'sparse')
            with self.assertRaisesRegex(ValueError, 'symlink'):
                cache.size(root)


if __name__ == '__main__':
    unittest.main()
