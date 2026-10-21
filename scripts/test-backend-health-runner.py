#!/usr/bin/env python3
"""Final-server smoke must execute the health command and propagate failures."""
import importlib.util
from pathlib import Path
import subprocess
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('backend_health', Path(__file__).with_name('check-backend-health.py'))
health = importlib.util.module_from_spec(spec)
spec.loader.exec_module(health)


class BackendHealth(unittest.TestCase):
    def exercise(self, *, probe_exit=0, running=True):
        calls = []
        def run(command, **kwargs):
            calls.append(command)
            output, error, code = '', '', 0
            if command[:3] == ['docker', 'image', 'inspect']: output = 'sha256:' + 'a' * 64 + '\n'
            elif command[:2] == ['docker', 'inspect']: output = 'true\n' if running else 'false\n'
            elif command[:2] == ['docker', 'exec']:
                if 'PORT=invalid' in command: code, error = 1, 'health-check: invalid PORT\n'
                else: code, error = probe_exit, 'health-check: connection failed\n' if probe_exit else ''
            elif command[0] == 'openssl':
                Path(command[command.index('-out') + 1]).write_text('disposable fixture')
            if code and kwargs.get('check'): raise subprocess.CalledProcessError(code, command)
            return subprocess.CompletedProcess(command, code, output, error)
        class Response:
            status = 200
            def __enter__(self): return self
            def __exit__(self, *args): pass
        with patch.object(health.subprocess, 'run', side_effect=run), patch.object(health, 'urlopen', return_value=Response()):
            if probe_exit or not running:
                with self.assertRaises((ValueError, subprocess.CalledProcessError)):
                    health.check('fixture', 'mongodb://127.0.0.1/fixture')
            else: health.check('fixture', 'mongodb://127.0.0.1/fixture')
        return calls

    def test_executes_real_binary_with_broken_config_and_preserves_image_user(self):
        calls = self.exercise()
        server = next(call for call in calls if call[:2] == ['docker', 'run'])
        self.assertNotIn('--user', server)
        self.assertIn('--read-only', server)
        probes = [call for call in calls if call[:2] == ['docker', 'exec']]
        self.assertEqual(len(probes), 2)
        self.assertEqual(probes[0][-2:], [health.BINARY, 'health-check'])
        for value in ('DATABASE_URL=invalid', 'ENCRYPTION_KEY=invalid', 'JWT_PRIVATE_KEY_PATH=/missing'):
            self.assertIn(value, probes[0])
        self.assertEqual(calls[-1][:4], ['docker', 'rm', '-f', '-v'])

    def test_failed_command_and_failed_server_always_block_and_cleanup(self):
        for kwargs in (dict(probe_exit=1), dict(running=False)):
            with self.subTest(**kwargs):
                calls = self.exercise(**kwargs)
                self.assertEqual(calls[-1][:4], ['docker', 'rm', '-f', '-v'])


if __name__ == '__main__': unittest.main()
