"""Read ELF64 little-endian runtime dependencies without executing the artifact."""
from pathlib import Path
import struct


def runtime(binary):
    data = Path(binary).read_bytes()
    if len(data) < 64 or data[:6] != b'\x7fELF\x02\x01':
        raise ValueError('expected a final ELF64 little-endian executable')
    offset = struct.unpack_from('<Q', data, 32)[0]
    size, count = struct.unpack_from('<HH', data, 54)
    if size != 56 or not 0 < count <= 128 or offset + count * size > len(data):
        raise ValueError('invalid ELF program headers')
    headers = [struct.unpack_from('<IIQQQQQQ', data, offset + i * size) for i in range(count)]
    needed, tables, interpreter = [], [], None
    for kind, _, start, address, _, length, _, _ in headers:
        if start + length > len(data): raise ValueError('truncated ELF segment')
        if kind == 3:
            if interpreter is not None: raise ValueError('multiple ELF interpreters')
            interpreter = data[start:start + length].rstrip(b'\0').decode('ascii')
        if kind == 2:
            if length % 16: raise ValueError('invalid ELF dynamic segment')
            for i in range(start, start + length, 16):
                tag, value = struct.unpack_from('<QQ', data, i)
                if tag == 0: break
                if tag == 1: needed.append(value)
                if tag == 5: tables.append(value)
    libraries = []
    if needed:
        if len(tables) != 1: raise ValueError('missing or ambiguous ELF dynamic string table')
        matches = [(start + tables[0] - address, start + length) for kind, _, start, address, _, length, _, _ in headers
                   if kind == 1 and address <= tables[0] < address + length]
        if len(matches) != 1: raise ValueError('invalid ELF dynamic string mapping')
        table, end = matches[0]
        for item in needed:
            if table + item >= end: raise ValueError('invalid ELF dependency offset')
            stop = data.find(b'\0', table + item, end)
            if stop < 0: raise ValueError('unterminated ELF dependency')
            name = data[table + item:stop].decode('ascii')
            if not name or '/' in name: raise ValueError('invalid ELF shared library name')
            libraries.append(name)
    return dict(interpreter=interpreter, needed=sorted(libraries))
