"""Discover every tracked Cargo/npm/pnpm lock and account for every manifest."""
import json
from pathlib import PurePosixPath
import subprocess


def discover(root):
    paths = set(subprocess.check_output(['git', 'ls-files', '-z'], cwd=root).decode().split('\0')) - {''}
    locks = sorted(p for p in paths if PurePosixPath(p).name in ('Cargo.lock', 'package-lock.json', 'pnpm-lock.yaml'))
    manifests = sorted(p for p in paths if PurePosixPath(p).name in ('Cargo.toml', 'package.json'))
    groups = {p: [] for p in locks}
    cargo_members = {}
    for lock in locks:
        if lock.endswith('Cargo.lock'):
            manifest = root / PurePosixPath(lock).parent / 'Cargo.toml'
            metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--locked', '--no-deps',
                                  '--format-version=1', '--manifest-path', str(manifest)], cwd=root))
            cargo_members[lock] = {str(__import__('pathlib').Path(p['manifest_path']).relative_to(root))
                                   for p in metadata['packages']}
            cargo_members[lock].add(str(PurePosixPath(lock).parent / 'Cargo.toml'))
    for name in manifests:
        path = root / name
        directory = PurePosixPath(name).parent
        if name.endswith('Cargo.toml'):
            candidates = [p for p in locks if p.endswith('Cargo.lock') and
                          directory.is_relative_to(PurePosixPath(p).parent)]
            # The audited workspace resolves this patch by path; its package
            # version and dependencies are in Cargo.lock. It is not a root.
            if name.startswith('vendor/'):
                if name != 'vendor/proc-macro-error2-2.0.1/Cargo.toml':
                    raise ValueError('unaccounted vendored manifest: ' + name)
            if not candidates:
                raise ValueError('unlocked Cargo manifest: ' + name)
        else:
            data = json.loads(path.read_text())
            # The browser extension's package.json is an authenticated Chrome
            # package identity, without npm package metadata or dependencies.
            if name == 'cli/resources/machine-browser/package.json' and set(data) == {'extension_id', 'version', 'sha256'}:
                continue
            candidates = [p for p in locks if not p.endswith('Cargo.lock') and
                          directory.is_relative_to(PurePosixPath(p).parent)]
            if not candidates:
                raise ValueError('unlocked Node manifest: ' + name)
        lock = max(candidates, key=lambda p: len(PurePosixPath(p).parts))
        if name.endswith('Cargo.toml') and not name.startswith('vendor/') and name not in cargo_members[lock]:
            raise ValueError('Cargo manifest is outside its audited workspace: ' + name)
        if not name.endswith('Cargo.toml') and directory != PurePosixPath(lock).parent:
            owner = json.loads((root / PurePosixPath(lock).parent / 'package.json').read_text())
            relative = str(directory.relative_to(PurePosixPath(lock).parent))
            import fnmatch
            workspaces = owner.get('workspaces', [])
            workspaces = workspaces.get('packages', []) if isinstance(workspaces, dict) else workspaces
            if not any(fnmatch.fnmatchcase(relative, pattern) for pattern in workspaces):
                raise ValueError('Node child is not in its locked workspace: ' + name)
        groups[lock].append(name)
    result = []
    for lock, covered in groups.items():
        if not covered:
            raise ValueError('orphan dependency lock: ' + lock)
        result.append(dict(lock=lock, manager='cargo' if lock.endswith('Cargo.lock') else
                           'pnpm' if lock.endswith('pnpm-lock.yaml') else 'npm', manifests=covered))
    return result
