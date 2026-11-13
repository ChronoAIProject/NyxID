import tempfile
import unittest
import subprocess
from unittest.mock import patch
from pathlib import Path

from compressed_swap import configure, prepare_statistics, statistics


class CompressedSwapTests(unittest.TestCase):
    def fixture(self, root):
        for name, value in {'enabled': 'N', 'compressor': 'lzo', 'max_pool_percent': '20'}.items():
            (root / name).write_text(value)

    def test_absent_interface_is_optional(self):
        with tempfile.TemporaryDirectory() as tmp:
            self.assertIn('unavailable', configure('zswap', Path(tmp))['status'])

    def test_zswap_and_off_and_effective_settings(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.fixture(root)
            writer = lambda path, value: path.write_text(value)
            self.assertEqual(configure('zswap', root, writer)['status'], 'configured')
            self.assertEqual((root / 'enabled').read_text(), 'Y')
            self.assertEqual((root / 'compressor').read_text(), 'zstd')
            self.assertEqual(configure('off', root, writer)['status'], 'configured')
            self.assertEqual((root / 'enabled').read_text(), 'N')

    def test_each_failed_write_rolls_back_and_never_raises(self):
        for failed in ('compressor', 'max_pool_percent', 'enabled'):
            with tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                self.fixture(root)
                original = {p.name: p.read_text() for p in root.iterdir()}
                did_fail = False
                def writer(path, value):
                    nonlocal did_fail
                    path.write_text(value)
                    if path.name == failed and not did_fail:
                        did_fail = True
                        raise OSError('test sysfs failure')
                result = configure('zswap', root, writer)
                self.assertIn('failed', result['status'])
                self.assertEqual({p.name: p.read_text() for p in root.iterdir()}, original)

    def test_pool_experiment_values_and_invalid_input(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.fixture(root)
            writer = lambda path, value: path.write_text(value)
            for value in ('20', '50'):
                result = configure('zswap', root, writer, pool_percent=value)
                self.assertEqual(result['status'], 'configured')
                self.assertEqual((root / 'max_pool_percent').read_text(), value)
            original = {p.name: p.read_text() for p in root.iterdir()}
            for value in ('0', '101', '-1', '20.5', '', 'twenty', '５０'):
                result = configure('zswap', root, writer, pool_percent=value)
                self.assertIn('invalid', result['status'])
                self.assertEqual({p.name: p.read_text() for p in root.iterdir()}, original)

    def test_statistics_mount_and_permission_failures_are_optional(self):
        with patch.dict('os.environ', {'RUNNER_ENVIRONMENT': 'github-hosted'}), \
             patch('compressed_swap.subprocess.run') as run:
            run.side_effect = [subprocess.CompletedProcess([], 1), subprocess.CompletedProcess([], 0)]
            self.assertEqual(prepare_statistics(), 'debugfs mounted')
            self.assertIn('mount', run.call_args.args[0])
            run.side_effect = OSError('unavailable')
            self.assertEqual(prepare_statistics(), 'debugfs unavailable')
            with patch('compressed_swap.read', return_value=None):
                self.assertTrue(all(v is None for v in statistics(privileged=True)['counters'].values()))

    def test_default_does_not_change_developer_swap(self):
        self.assertEqual(configure('unchanged')['status'], 'unchanged')


if __name__ == '__main__':
    unittest.main()
