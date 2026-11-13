#!/usr/bin/env python3
"""Bound CI graphs and retain sessions produced after the build marker."""

import argparse
from contextlib import ExitStack
import fcntl
from pathlib import Path
import re
import shutil
import subprocess

ROOTS = (Path('target/debug/incremental'), Path('target/llvm-cov-target/debug/incremental'))
MARKER = Path('target/ci-incremental-build-start')
MAX_BYTES = 16 * 1024**3


def mark():
    MARKER.parent.mkdir(parents=True, exist_ok=True)
    MARKER.write_text('')
    print(f'[ci-incremental] build marker: {MARKER.stat().st_mtime_ns}', flush=True)


def size(path):
    # Logical bytes plus a conservative tar/header/block allowance. Do not
    # underestimate sparse files or count hardlinks only once.
    if path.is_symlink():
        raise ValueError(f'incremental cache contains a symlink: {path}')
    total = 4096
    for entry in path.rglob('*'):
        if entry.is_symlink():
            raise ValueError(f'incremental cache contains a symlink: {entry}')
        total += (entry.stat().st_size if entry.is_file() else 0) + 4096
    return total


def inventory(label):
    subprocess.run(['df', '-h', '.'], check=True)
    total = 0
    for root in ROOTS:
        if root.is_symlink():
            raise ValueError(f'incremental root is a symlink: {root}')
        if not root.exists():
            continue
        for crate in sorted(root.iterdir()):
            if not crate.is_dir() or crate.is_symlink():
                raise ValueError(f'unexpected incremental cache entry: {crate}')
            amount = size(crate)
            total += amount
            print(f'[ci-incremental] {label} crate {crate}: {amount} bytes', flush=True)
            for session in sorted(crate.iterdir()):
                if session.is_dir():
                    print(f'[ci-incremental] session {session.name}: {size(session)} bytes', flush=True)
    print(f'[ci-incremental] {label} total: {total} bytes', flush=True)
    return total


SESSION = re.compile(r's-([0-9a-z]+)-([0-9a-z]+)-([0-9a-z]+|working)')
LOCK = re.compile(r's-[0-9a-z]+-[0-9a-z]+\.lock')


def prune_sessions(crate, changed_since=None):
    """Keep exactly the finalized session rustc would load on the next build.

    Rustc chooses the greatest base-36 timestamp encoded in the session name.
    Its GC can leave the prior session, which was locked as the current build's
    input. Prune only after Cargo exits and acquire every session lock first.
    """
    with ExitStack() as locks:
        entries = list(crate.iterdir())
        lock_names = set()
        sessions = []
        for entry in entries:
            if entry.is_file() and LOCK.fullmatch(entry.name):
                lock = locks.enter_context(entry.open('rb'))
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
                lock_names.add(entry.name)
            elif entry.is_dir() and (match := SESSION.fullmatch(entry.name)):
                sessions.append((int(match[1], 36), entry, match))
            else:
                raise ValueError(f'unexpected rustc session entry: {entry}')
        finalized = [(stamp, entry) for stamp, entry, match in sessions if match[3] != 'working']
        for _, entry, match in sessions:
            if f's-{match[1]}-{match[2]}.lock' not in lock_names:
                raise ValueError(f'missing rustc session lock: {entry}')
        if changed_since is not None and not any(entry.stat().st_mtime_ns > changed_since
                                                 for _, entry in finalized):
            print(f'[ci-incremental] remove unused crate {crate}', flush=True)
            shutil.rmtree(crate)
            return
        if not finalized:
            raise ValueError(f'no finalized rustc session: {crate}')
        latest = max(stamp for stamp, _ in finalized)
        candidates = [entry for stamp, entry in finalized if stamp == latest]
        if len(candidates) != 1:
            raise ValueError(f'ambiguous latest rustc session: {crate}')
        keep = candidates[0]
        keep_lock = '-'.join(keep.name.split('-')[:3]) + '.lock'
        for _, entry, _ in sessions:
            if entry != keep:
                print(f'[ci-incremental] remove old session {entry}', flush=True)
                shutil.rmtree(entry)
        for name in lock_names - {keep_lock}:
            (crate / name).unlink()


def prune():
    if not MARKER.is_file():
        raise ValueError('no build marker; refusing to save incremental cache')
    changed_since = MARKER.stat().st_mtime_ns
    before = inventory('before-prune')
    for root in ROOTS:
        if root.exists():
            for crate in root.iterdir():
                prune_sessions(crate, changed_since)
    after = inventory('after-prune')
    print(f'[ci-incremental] pruned {before - after} bytes', flush=True)
    if after == 0:
        raise ValueError('empty graph after pruning; not saving')
    if after > MAX_BYTES:
        raise ValueError(f'pruned graph exceeds the {MAX_BYTES}-byte restore contract; not saving')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=('mark', 'inventory', 'prune'))
    parser.add_argument('--label', default='after-restore')
    args = parser.parse_args()
    if args.command == 'mark':
        mark()
    elif args.command == 'prune':
        prune()
    else:
        inventory(args.label)


if __name__ == '__main__':
    main()
