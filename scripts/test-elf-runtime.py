#!/usr/bin/env python3
"""ELF dependency extraction on static, dynamic and malformed executables."""
from pathlib import Path
import struct
import tempfile
import unittest
from elf_runtime import runtime

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


class ElfRuntime(unittest.TestCase):
    def test_static_dynamic_and_interpreter(self):
        with tempfile.TemporaryDirectory() as temporary:
            p = Path(temporary) / 'binary'
            p.write_bytes(elf()); self.assertEqual(runtime(p), dict(interpreter=None, needed=[]))
            p.write_bytes(elf(['libdbus-1.so.3', 'libc.so.6'], '/lib/ld-linux.so'))
            self.assertEqual(runtime(p), dict(interpreter='/lib/ld-linux.so', needed=['libc.so.6', 'libdbus-1.so.3']))

    def test_malformed_segments_strings_and_headers_fail(self):
        with tempfile.TemporaryDirectory() as temporary:
            p = Path(temporary) / 'binary'
            data = elf(['libc.so.6'])
            malformed = bytearray(data); struct.pack_into('<Q', malformed, 600, 4096)
            for content in (b'not ELF', data[:90], data[:800], malformed):
                p.write_bytes(content)
                with self.subTest(size=len(content)), self.assertRaises(ValueError): runtime(p)

if __name__ == '__main__': unittest.main()
