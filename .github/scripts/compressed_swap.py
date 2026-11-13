#!/usr/bin/env python3
"""Best-effort zswap experiment; the disk-backed 40 GiB swap remains mandatory."""
import argparse
import json
import os
from pathlib import Path
import subprocess

PARAMETERS = Path('/sys/module/zswap/parameters')
STATS = Path('/sys/kernel/debug/zswap')
NAMES = ('enabled', 'compressor', 'max_pool_percent', 'zpool', 'accept_threshold_percent')
COUNTERS = ('pool_total_size', 'stored_pages', 'same_filled_pages', 'written_back_pages',
            'pool_limit_hit', 'reject_compress_poor', 'reject_alloc_fail', 'reject_reclaim_fail')


def read(path):
    try:
        return path.read_text().strip()
    except (OSError, UnicodeError):
        return None


def write(path, value):
    result = subprocess.run(['sudo', '-n', 'tee', str(path)], input=value + '\n',
                            capture_output=True, text=True, timeout=5)
    if result.returncode:
        raise OSError(f'write failed: {path}')


def configure(mode, root=PARAMETERS, writer=write, pool_percent='20'):
    if mode == 'zswap' and (not str(pool_percent).isascii() or not str(pool_percent).isdigit()
                            or not 1 <= int(pool_percent) <= 100):
        return {'status': 'invalid pool percent; unchanged', 'mode': mode,
                'pool_percent': pool_percent}
    desired = {'compressor': 'zstd', 'max_pool_percent': str(pool_percent), 'enabled': 'Y'} if mode == 'zswap' else {'enabled': 'N'}
    if mode not in {'off', 'zswap'}:
        return {'status': 'unchanged', 'mode': mode}
    original = {name: read(root / name) for name in desired}
    if any(value is None for value in original.values()):
        return {'status': 'unavailable; unchanged', 'mode': mode}
    changed = []
    try:
        for name, value in desired.items():
            # Include a write that fails after a partial effect in rollback.
            changed.append(name)
            writer(root / name, value)
        return {'status': 'configured', 'mode': mode, 'requested': desired}
    except (OSError, subprocess.SubprocessError) as error:
        rollback_errors = []
        for name in reversed(changed):
            try:
                writer(root / name, original[name])
            except (OSError, subprocess.SubprocessError):
                rollback_errors.append(name)
        return {'status': ('configuration failed; rollback incomplete' if rollback_errors else
                           'configuration failed; restored prior settings'), 'mode': mode,
                'error': str(error), 'rollback_errors': rollback_errors}


def prepare_statistics():
    """Expose counters on ephemeral runners when debugfs is not already mounted."""
    if os.environ.get('RUNNER_ENVIRONMENT') != 'github-hosted':
        return 'mount unchanged outside hosted runner'
    try:
        mounted = subprocess.run(['findmnt', '-n', '--mountpoint', '/sys/kernel/debug'],
                                 capture_output=True, timeout=5)
        if mounted.returncode == 0:
            return 'debugfs already mounted'
        if mounted.returncode != 1:
            return 'debugfs mount status unavailable'
        result = subprocess.run(['sudo', '-n', 'mount', '-t', 'debugfs', 'debugfs', '/sys/kernel/debug'],
                                capture_output=True, timeout=5)
        return 'debugfs mounted' if result.returncode == 0 else 'debugfs mount unavailable'
    except (OSError, subprocess.SubprocessError):
        return 'debugfs unavailable'


def statistics(privileged=False):
    result = {'parameters': {name: read(PARAMETERS / name) for name in NAMES},
              'vm': {name: read(Path('/proc/sys/vm') / name) for name in ('swappiness', 'page-cluster')}}
    counters = {}
    for name in COUNTERS:
        value = read(STATS / name)
        if value is None and privileged:
            try:
                command = subprocess.run(['sudo', '-n', 'cat', str(STATS / name)],
                                         capture_output=True, text=True, timeout=5)
                value = command.stdout.strip() if command.returncode == 0 else None
            except (OSError, subprocess.SubprocessError):
                pass
        counters[name] = value
    result['counters'] = counters
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=('configure', 'stats'))
    args = parser.parse_args()
    if args.command == 'configure':
        mode = os.environ.get('NYXID_SWAP_MODE', 'unchanged')
        print('[ci-zswap] ' + json.dumps(configure(mode, pool_percent=os.environ.get('NYXID_ZSWAP_POOL_PERCENT', '20'))))
        if mode in {'off', 'zswap'}:
            print('[ci-zswap] ' + prepare_statistics())
    privileged = args.command == 'stats' or os.environ.get('NYXID_SWAP_MODE') in {'off', 'zswap'}
    print('[ci-zswap] ' + json.dumps(statistics(privileged=privileged)))


if __name__ == '__main__':
    # Optional acceleration must never prevent the existing swap setup/build.
    try:
        main()
    except Exception as error:
        print('[ci-zswap] unavailable; continuing with disk swap: ' + str(error))
