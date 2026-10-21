#!/usr/bin/env python3
"""Native keychain gate must execute its test and propagate every failure."""
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('native', Path(__file__).with_name('test-native-secret-service.py'))
native = importlib.util.module_from_spec(spec); spec.loader.exec_module(native)


class NativeGate(unittest.TestCase):
    def test_absent_prerequisite_cannot_skip(self):
        with patch.object(native.shutil, 'which', return_value=None), self.assertRaisesRegex(ValueError, 'prerequisite missing'):
            native.main()

    def test_test_presence_and_real_execution_with_isolated_session_are_required(self):
        with tempfile.TemporaryDirectory() as temporary:
            binary = Path(temporary).resolve() / 'actual-cli-test'; binary.write_text('fixture')
            artifact = dict(reason='compiler-artifact', executable=str(binary), target=dict(name='nyxid'), profile=dict(test=True))
            build = subprocess.CompletedProcess([], 0, json.dumps(artifact) + '\n', '')
            for outcome in ('passed', 'missing-test', 'failed', 'deadlock'):
                def run(command, **kwargs):
                    if command[0] == 'cargo':
                        self.assertIn('--locked', command); self.assertIn('--no-run', command)
                        return build
                    self.assertEqual(command[:4], ['dbus-run-session', '--', 'sh', '-ec'])
                    self.assertEqual(command[-2:], [str(binary), native.TEST])
                    self.assertEqual(kwargs['timeout'], 90)
                    self.assertTrue(kwargs['check'])
                    self.assertNotIn('DBUS_SESSION_BUS_ADDRESS', kwargs['env'])
                    for name in ('XDG_RUNTIME_DIR', 'XDG_CONFIG_HOME', 'XDG_DATA_HOME'):
                        self.assertTrue(Path(kwargs['env'][name]).is_dir())
                    if outcome == 'failed': raise subprocess.CalledProcessError(1, command)
                    if outcome == 'deadlock': raise subprocess.TimeoutExpired(command, 90)
                    return subprocess.CompletedProcess(command, 0)
                listing = '' if outcome == 'missing-test' else native.TEST + ': test\n'
                with self.subTest(outcome=outcome), patch.object(native.shutil, 'which', return_value='/usr/bin/fixture'), \
                        patch.object(native.subprocess, 'run', side_effect=run) as runner, \
                        patch.object(native.subprocess, 'check_output', return_value=listing):
                    if outcome == 'passed':
                        native.main(); self.assertEqual(runner.call_count, 2)
                    else:
                        with self.assertRaises((ValueError, subprocess.CalledProcessError, subprocess.TimeoutExpired)):
                            native.main()
                        self.assertEqual(runner.call_count, 1 if outcome == 'missing-test' else 2)


if __name__ == '__main__': unittest.main()
