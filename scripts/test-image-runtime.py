#!/usr/bin/env python3
"""Final ELF dependency, real-entrypoint invocation and failure propagation tests."""
import importlib.util
import json
from pathlib import Path
import struct
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from elf_runtime import runtime
import image_runtime


def elf(needed=(), interpreter=None):
    headers = []
    data = bytearray(1024); data[:6] = b'\x7fELF\x02\x01'
    struct.pack_into('<HH', data, 16, 3, 183); struct.pack_into('<Q', data, 32, 64)
    headers.append((1, 4, 0, 0x10000, 0, len(data), len(data), 4096))
    if interpreter:
        encoded = interpreter.encode() + b'\0'; data[512:512 + len(encoded)] = encoded
        headers.append((3, 4, 512, 0, 0, len(encoded), len(encoded), 1))
    if needed:
        strings = b'\0'; offsets = []
        for name in needed: offsets.append(len(strings)); strings += name.encode() + b'\0'
        data[768:768 + len(strings)] = strings
        tags = [(5, 0x10000 + 768)] + [(1, value) for value in offsets] + [(0, 0)]
        for i, (tag, value) in enumerate(tags): struct.pack_into('<QQ', data, 576 + i * 16, tag, value)
        headers.append((2, 4, 576, 0, 0, len(tags) * 16, len(tags) * 16, 8))
    struct.pack_into('<HH', data, 54, 56, len(headers))
    for i, header in enumerate(headers): struct.pack_into('<IIQQQQQQ', data, 64 + i * 56, *header)
    return data


class Runtime(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(); self.addCleanup(self.tmp.cleanup)
        self.binary = Path(self.tmp.name) / 'nyxid'; self.binary.write_bytes(elf())
        self.image = 'sha256:' + 'a' * 64

    def test_reads_static_and_dynamic_dependencies_without_executing_binary(self):
        self.assertEqual(runtime(self.binary), dict(interpreter=None, needed=[]))
        self.binary.write_bytes(elf(['libdbus-1.so.3', 'libc.so.6'], '/lib/ld-linux.so'))
        self.assertEqual(runtime(self.binary), dict(interpreter='/lib/ld-linux.so', needed=['libc.so.6', 'libdbus-1.so.3']))

    def test_malformed_or_truncated_elf_fails(self):
        data = elf(['libc.so.6'])
        for changed in (b'not ELF', data[:90], data[:800]):
            self.binary.write_bytes(changed)
            with self.subTest(size=len(changed)), self.assertRaises(ValueError): runtime(self.binary)

    def test_dbUS_is_forbidden_in_the_five_package_node_rootfs(self):
        self.binary.write_bytes(elf(['libdbus-1.so.3']))
        with patch.object(image_runtime, 'probe') as probe, self.assertRaisesRegex(ValueError, 'undeclared shared libraries'):
            image_runtime.check(self.image, 'node-agent', self.binary)
        self.assertEqual(probe.call_args.args[:3], (self.image, '/usr/local/bin/nyxid', ['--version']))

    def test_scratch_with_needed_library_or_interpreter_fails_before_execution(self):
        for component in ('mcp-demo', 'nyxid-machine-updater'):
            for data in (elf(['libc.so.6']), elf(interpreter='/lib/ld-linux.so')):
                self.binary.write_bytes(data)
                with self.subTest(component=component), patch.object(image_runtime, 'probe') as probe, self.assertRaisesRegex(ValueError, 'not a static'):
                    image_runtime.check(self.image, component, self.binary)
                probe.assert_not_called()

    def test_all_cli_images_execute_version_help_and_the_real_entrypoint(self):
        for component in ('node-agent', 'nyxid-node-machine', 'machine-node-only', 'machine-hosted-desktop', 'machine-hosted'):
            with self.subTest(component=component), patch.object(image_runtime, 'probe') as probe:
                image_runtime.check(self.image, component, self.binary)
                self.assertEqual([c.args[:3] for c in probe.call_args_list], [
                    (self.image, '/usr/local/bin/nyxid', ['--version']),
                    (self.image, '/usr/local/bin/nyxid', ['--help']), (self.image, None, ['--help'])])

    def test_mcp_real_stdio_protocol_response_is_required(self):
        for response in ('', '{}', '{"jsonrpc":"2.0","id":1,"result":{}}'):
            with patch.object(image_runtime, 'probe', return_value=subprocess.CompletedProcess([], 0, response)):
                if response.startswith('{"jsonrpc"'): image_runtime.check(self.image, 'mcp-demo', self.binary)
                else:
                    with self.assertRaises(ValueError): image_runtime.check(self.image, 'mcp-demo', self.binary)

    def test_missing_library_exit127_wrong_help_and_nonzero_all_block_and_cleanup(self):
        for code, output in ((127, 'libdbus-1.so.3: cannot open shared object file'), (0, ''), (1, 'Usage:')):
            def run(command, **kwargs): return subprocess.CompletedProcess(command, code, '', output)
            with patch.object(image_runtime.subprocess, 'run', side_effect=run) as run, self.assertRaisesRegex(ValueError, 'runtime probe failed'):
                image_runtime.probe(self.image, None, ['--help'], required='Usage:')
            command = run.call_args_list[0].args[0]
            self.assertIn('--network=none', command); self.assertIn('--read-only', command)
            self.assertIn('--cap-drop=ALL', command)
            self.assertNotIn('-e', command); self.assertNotIn('-v', command); self.assertNotIn('--mount', command)
            self.assertEqual(run.call_args_list[-1].args[0][:5], ['docker', 'rm', '-f', '-v', command[4]])

    def test_timeout_still_removes_probe_and_anonymous_volumes(self):
        def run(command, **kwargs):
            if command[1] == 'run': raise subprocess.TimeoutExpired(command, 60)
            return subprocess.CompletedProcess(command, 0)
        with patch.object(image_runtime.subprocess, 'run', side_effect=run) as run, self.assertRaises(subprocess.TimeoutExpired):
            image_runtime.probe(self.image, None, ['--help'])
        self.assertEqual(run.call_args_list[-1].args[0][1:4], ['rm', '-f', '-v'])

    def test_identity_checker_cannot_succeed_after_a_failed_runtime_probe(self):
        spec = importlib.util.spec_from_file_location('image_identity', Path(__file__).with_name('check-image-identity.py'))
        identity = importlib.util.module_from_spec(spec); spec.loader.exec_module(identity)
        from contextlib import contextmanager
        @contextmanager
        def container(image): yield 'fixture-container', {'Id': self.image}
        with patch.object(identity, 'image_container', container), patch.object(identity, 'verify_labels'), \
                patch.object(identity, 'copy', return_value=['nyxid-mcp-demo']), patch.object(identity, 'versions', return_value={'nyxid-mcp-demo': '1'}), \
                patch.object(identity, 'verify_binary'), patch.object(identity, 'check_runtime', side_effect=ValueError('runtime failed')) as probe:
            with self.assertRaisesRegex(ValueError, 'runtime failed'):
                identity.check('tag', 'mcp-demo', 'arm64', Path('/source'), 'a'*40, 1, Path('/extractor'))
            self.assertEqual(probe.call_count, 1)
            self.assertEqual(probe.call_args.args[:2], (self.image, 'mcp-demo'))


if __name__ == '__main__': unittest.main()
