#!/usr/bin/env python3
"""Run actual shell dispatch without entering machine registration/startup."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

SCRIPT = Path(__file__).with_name('image-entrypoint-dispatch.sh')


class Dispatch(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(); self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        (self.root / 'nyxid').write_text('#!/bin/sh\nprintf "%s\\n" "$@"\n')
        (self.root / 'nyxid').chmod(0o755)

    def run_script(self, *args):
        return subprocess.run(['sh', str(SCRIPT), *args], env=dict(os.environ, PATH=str(self.root) + ':' + os.environ['PATH']),
                              capture_output=True, text=True)

    def test_sole_help_or_version_execs_only_cli_for_both_fixed_entrypoints(self):
        for entrypoint in ('/usr/local/bin/nyxid-machine-entrypoint', '/usr/local/bin/nyxid-machine-node-only'):
            for flag in ('--help', '--version'):
                with self.subTest(entrypoint=entrypoint, flag=flag):
                    r = self.run_script(entrypoint, flag)
                    self.assertEqual((r.returncode, r.stdout, r.stderr), (0, flag + '\n', ''))

    def test_missing_or_arbitrary_entrypoint_fails(self):
        self.assertEqual(self.run_script().returncode, 2)
        for entrypoint in ('/bin/sh', '/tmp/command', '/usr/local/bin/nyxid-machine-node-only/../other'):
            self.assertEqual(self.run_script(entrypoint, '--help').returncode, 2)

    def test_extra_or_ordinary_arguments_go_to_original_entrypoint(self):
        # These absolute production entrypoints do not exist on the test host.
        # A failed exec of that fixed path proves no CLI help shortcut was taken.
        for args in ((), ('--help', '--token'), ('--token', '--help'), ('--version=x',)):
            r = self.run_script('/usr/local/bin/nyxid-machine-node-only', *args)
            self.assertNotEqual(r.returncode, 0)
            self.assertIn('/usr/local/bin/nyxid-machine-node-only', r.stderr)
            self.assertEqual(r.stdout, '')


if __name__ == '__main__': unittest.main()
