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
# Even small lanes can restore any graph up to the shared writer cap. Budget
# another capped graph plus 8 GiB for objects/binaries, rather than extrapolate
# from the current small graph sizes.
BUILD_GROWTH_GIB = GRAPH_GIB + 8
TEST_ARCHIVE_GIB = 4
FLOOR_GIB = 8


def reserve_bytes(meminfo):
    swap_kib = next(int(line.split()[1]) for line in meminfo.splitlines()
                    if line.startswith('SwapTotal:'))
    missing_kib = max(0, 40 * 1024**2 - swap_kib)
    return ((missing_kib + 1023) // 1024 + 1) * 1024**2 if missing_kib else 0


def restore_required(swap_bytes, *, backend_reserve=False):
    swap_bytes = swap_bytes if backend_reserve else 0
    # Match prepare-backend-runner.sh's archive allowance in reserve jobs.
    # Non-reserve graph consumers never create a nextest archive.
    archive_gib = TEST_ARCHIVE_GIB if backend_reserve else 0
    return max((GRAPH_GIB + ARCHIVE_GIB + FLOOR_GIB) * GIB,
               swap_bytes + (GRAPH_GIB + BUILD_GROWTH_GIB + archive_gib + FLOOR_GIB) * GIB)


def best_effort(command):
    """Cleanup and diagnostics are optional; measured free space governs admission."""
    try:
        subprocess.run(command, check=True)
    except (OSError, subprocess.SubprocessError) as error:
        print(f'::warning::Optional disk preparation failed ({" ".join(command)}): {error}', flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('phase', choices=('before-dependencies', 'before-incremental', 'before-save'))
    parser.add_argument('--backend-reserve', choices=('true', 'false'), default='false',
                        help='This job will run prepare-backend-runner.sh after restore.')
    args = parser.parse_args()
    if args.phase == 'before-save':
        subprocess.run(['df', '-h', '.'], check=True)
    else:
        best_effort(['df', '-h', '.'])
    if args.phase == 'before-dependencies':
        # Only reserve-allocating jobs opt into this phase through the action.
        # Never remove host software outside an ephemeral GitHub runner.
        if os.environ.get('RUNNER_ENVIRONMENT') == 'github-hosted' and os.environ.get('RUNNER_OS') == 'Linux':
            best_effort(['sudo', '-n', 'rm', '-rf', '/usr/local/lib/android',
                         '/usr/share/dotnet', '/opt/ghc', '/usr/local/.ghcup'])
            best_effort(['df', '-h', '.'])
        return
    free = shutil.disk_usage('.').free
    if args.phase == 'before-save':
        required = (ARCHIVE_GIB + FLOOR_GIB) * GIB
        if free < required:
            raise SystemExit(f'Cache save requires {required} free bytes; have {free}; not saving')
        return
    backend_reserve = args.backend_reserve == 'true'
    swap = 0
    if backend_reserve and Path('/proc/meminfo').exists():
        swap = reserve_bytes(Path('/proc/meminfo').read_text())
    required = restore_required(swap, backend_reserve=backend_reserve)
    allowed = free >= required
    print(f'[ci-disk] free={free} required={required} restore={allowed}; '
          f'graph<={GRAPH_GIB} GiB, download<={ARCHIVE_GIB} GiB, added swap={swap}, '
          f'backend reserve={backend_reserve}, build growth={BUILD_GROWTH_GIB} GiB, '
          f'test archive={TEST_ARCHIVE_GIB if backend_reserve else 0} GiB, floor={FLOOR_GIB} GiB', flush=True)
    with open(os.environ['GITHUB_OUTPUT'], 'a', encoding='utf-8') as out:
        out.write(f'allowed={str(allowed).lower()}\n')
    if not allowed:
        print('::warning::Skipping incremental restore: insufficient pre-extraction disk budget.')


if __name__ == '__main__':
    main()
