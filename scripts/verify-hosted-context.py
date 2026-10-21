#!/usr/bin/env python3
"""Verify every reviewed CMA patch before executing any of their bytes.

BuildKit supplies the external context as a read-only source snapshot. Execute
only private temporary copies of the bytes just verified, preserving sibling
require() behavior without retaining private source in an image layer.
"""
import argparse
import hashlib
from pathlib import Path
import re
import subprocess
import tempfile

NAMES = ('persist_conversations.cjs', 'subscription_auth.cjs', 'managed_model_catalog.cjs')


def verified(context, pins):
    if context.is_symlink() or pins.is_symlink():
        raise ValueError('context and pins must not be symlinked')
    lines = pins.read_text().splitlines()
    if len(lines) != 5 or lines[0] != '# nyxid.hosted-context.v1' or not re.fullmatch('# source-commit [0-9a-f]{40}', lines[1]):
        raise ValueError('invalid hosted context pin manifest')
    data = {}
    for name, line in zip(NAMES, lines[2:]):
        fields = line.split('\t')
        if len(fields) != 2 or fields[0] != name or not re.fullmatch('[0-9a-f]{64}', fields[1]):
            raise ValueError('hosted context requires exact canonical filenames and order')
        source = context / name
        if source.is_symlink() or not source.is_file() or source.stat().st_size > 256 * 1024:
            raise ValueError('hosted patch is missing, symlinked or too large: ' + name)
        value = source.read_bytes()
        if hashlib.sha256(value).hexdigest() != fields[1]:
            raise ValueError('hosted patch checksum mismatch: ' + name)
        data[name] = value
    return data


def verify(context, pins, execute=False):
    data = verified(context, pins)
    print('All three hosted patches match reviewed bytes and order.', flush=True)
    if execute:
        with tempfile.TemporaryDirectory(prefix='nyxid-verified-hosted-') as directory:
            for name, value in data.items():
                (Path(directory) / name).write_bytes(value)
            for name in NAMES:
                subprocess.run(['node', str(Path(directory) / name)], check=True, cwd=directory)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--context', type=Path, required=True)
    parser.add_argument('--pins', type=Path, required=True)
    modes = parser.add_mutually_exclusive_group(required=True)
    modes.add_argument('--verify-only', action='store_true')
    modes.add_argument('--execute', action='store_true')
    args = parser.parse_args()
    verify(args.context, args.pins, args.execute)
