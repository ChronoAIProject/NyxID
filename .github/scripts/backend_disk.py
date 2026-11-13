#!/usr/bin/env python3
"""Check restore/build disk budgets before cache extraction can kill the runner."""
import argparse
import os
from pathlib import Path
import shutil
import subprocess

from incremental_cache import MAX_BYTES

GIB = 1024**3
# A saved graph is capped at 16 GiB including tar overhead. zstd's worst-case
# archive fits 17 GiB. Compilation can hold the restored graph and its successor.
GRAPH_GIB = MAX_BYTES // GIB
ARCHIVE_GIB = GRAPH_GIB + 1
BUILD_GROWTH_GIB = 24
TEST_ARCHIVE_GIB = 4
FLOOR_GIB = 8


def reserve_bytes(meminfo):
    swap_kib = next(int(line.split()[1]) for line in meminfo.splitlines()
                    if line.startswith('SwapTotal:'))
    missing_kib = max(0, 40 * 1024**2 - swap_kib)
    return ((missing_kib + 1023) // 1024 + 1) * 1024**2 if missing_kib else 0


def restore_required(swap_bytes):
    return max((GRAPH_GIB + ARCHIVE_GIB + FLOOR_GIB) * GIB,
               swap_bytes + (GRAPH_GIB + BUILD_GROWTH_GIB + TEST_ARCHIVE_GIB + FLOOR_GIB) * GIB)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('phase', choices=('before-dependencies', 'before-incremental', 'before-save'))
    args = parser.parse_args()
    subprocess.run(['df', '-h', '.'], check=True)
    if args.phase == 'before-dependencies':
        # These SDKs are unused by the Rust jobs that call this composite.
        # Never remove host software outside an ephemeral GitHub runner.
        if os.environ.get('RUNNER_ENVIRONMENT') == 'github-hosted' and os.environ.get('RUNNER_OS') == 'Linux':
            subprocess.run(['sudo', '-n', 'rm', '-rf', '/usr/local/lib/android',
                            '/usr/share/dotnet', '/opt/ghc', '/usr/local/.ghcup'], check=True)
            subprocess.run(['df', '-h', '.'], check=True)
        return
    free = shutil.disk_usage('.').free
    if args.phase == 'before-save':
        required = (ARCHIVE_GIB + FLOOR_GIB) * GIB
        if free < required:
            raise SystemExit(f'Cache save requires {required} free bytes; have {free}; not saving')
        return
    swap = reserve_bytes(Path('/proc/meminfo').read_text()) if Path('/proc/meminfo').exists() else 0
    required = restore_required(swap)
    allowed = free >= required
    print(f'[ci-disk] free={free} required={required} restore={allowed}; '
          f'graph<={GRAPH_GIB} GiB, download<={ARCHIVE_GIB} GiB, added swap={swap}, '
          'build growth=24 GiB, test archive=4 GiB, floor=8 GiB', flush=True)
    with open(os.environ['GITHUB_OUTPUT'], 'a', encoding='utf-8') as out:
        out.write(f'allowed={str(allowed).lower()}\n')
    if not allowed:
        print('::warning::Skipping incremental restore: insufficient pre-extraction disk budget.')


if __name__ == '__main__':
    main()
