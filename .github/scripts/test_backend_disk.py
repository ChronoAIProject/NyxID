import io
from contextlib import redirect_stdout
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import backend_disk


class BackendDiskTests(unittest.TestCase):
    def test_reclamation_never_removes_sdks_outside_hosted_linux(self):
        for environment, platform in (('self-hosted', 'Linux'), ('github-hosted', 'macOS'), ('', '')):
            with self.subTest(environment=environment, platform=platform), \
                 patch.dict(os.environ, {'RUNNER_ENVIRONMENT': environment, 'RUNNER_OS': platform}), \
                 patch.object(sys, 'argv', ['backend_disk', 'before-dependencies']), \
                 patch('backend_disk.subprocess.run') as run:
                backend_disk.main()
                run.assert_called_once_with(['df', '-h', '.'], check=True)

    def test_failed_reclamation_warns_and_free_space_still_controls_restore(self):
        for failure in (OSError('sudo unavailable'), subprocess.CalledProcessError(1, 'rm')):
            with self.subTest(failure=failure), tempfile.TemporaryDirectory() as tmp:
                output = Path(tmp) / 'output'
                diagnostics = io.StringIO()
                with patch.dict(os.environ, {'RUNNER_ENVIRONMENT': 'github-hosted', 'RUNNER_OS': 'Linux',
                                             'GITHUB_OUTPUT': str(output)}), \
                     patch.object(sys, 'argv', ['backend_disk', 'before-dependencies']), \
                     patch('backend_disk.subprocess.run', side_effect=[None, failure, None]) as run, \
                     redirect_stdout(diagnostics):
                    backend_disk.main()
                self.assertEqual(run.call_args_list[1].args[0][:4], ['sudo', '-n', 'rm', '-rf'])
                self.assertEqual(run.call_args_list[-1].args[0], ['df', '-h', '.'])
                self.assertIn('::warning::Optional disk preparation failed (sudo', diagnostics.getvalue())
                # A failed or partial deletion never grants restore permission.
                for free, allowed in ((0, 'false'), (200 * backend_disk.GIB, 'true')):
                    output.write_text('')
                    with patch.dict(os.environ, {'GITHUB_OUTPUT': str(output)}), \
                         patch.object(sys, 'argv', ['backend_disk', 'before-incremental']), \
                         patch('backend_disk.shutil.disk_usage') as usage, \
                         patch('backend_disk.subprocess.run'), redirect_stdout(io.StringIO()):
                        usage.return_value.free = free
                        backend_disk.main()
                    self.assertEqual(output.read_text(), f'allowed={allowed}\n')

    def test_failed_df_does_not_fail_cleanup_or_bypass_restore_guard(self):
        with tempfile.TemporaryDirectory() as tmp:
            output = Path(tmp) / 'output'
            for phase in ('before-dependencies', 'before-incremental'):
                diagnostics = io.StringIO()
                with self.subTest(phase=phase), \
                     patch.dict(os.environ, {'RUNNER_ENVIRONMENT': 'self-hosted', 'GITHUB_OUTPUT': str(output)}), \
                     patch.object(sys, 'argv', ['backend_disk', phase]), \
                     patch('backend_disk.shutil.disk_usage') as usage, \
                     patch('backend_disk.subprocess.run', side_effect=subprocess.CalledProcessError(1, 'df')), \
                     redirect_stdout(diagnostics):
                    usage.return_value.free = 0
                    backend_disk.main()
                self.assertIn('::warning::Optional disk preparation failed (df', diagnostics.getvalue())
            self.assertEqual(output.read_text(), 'allowed=false\n')

    def test_writer_preflight_stays_strict(self):
        with patch.object(sys, 'argv', ['backend_disk', 'before-save']), \
             patch('backend_disk.subprocess.run', side_effect=subprocess.CalledProcessError(1, 'df')):
            with self.assertRaises(subprocess.CalledProcessError):
                backend_disk.main()


if __name__ == '__main__':
    unittest.main()
