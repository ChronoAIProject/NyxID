#!/usr/bin/env python3
"""Scan all reachable history and the committed tree; print no credential bytes."""
import argparse
from collections import Counter
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile

from secret_policy import disposition, finding_identity, load_exceptions, normalized_path

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('secret_source_export', ROOT / 'scripts/export-build-source.py')
source_export = importlib.util.module_from_spec(spec)
spec.loader.exec_module(source_export)


def git(root, *args):
    return source_export.git(root, *args)


def scan(root, scanner, output):
    if git(root, 'rev-parse', '--is-shallow-repository').strip() != b'false':
        raise ValueError('full-history scanning requires a complete checkout (fetch-depth: 0)')
    entries = load_exceptions(root / 'security/secret-exceptions.json')
    output.mkdir(parents=True, exist_ok=True)
    findings = []
    counts = Counter()
    # Raw scanner output is temporary and private. CI artifacts contain only
    # locations and content identities. Neither logs nor artifacts contain keys.
    with tempfile.TemporaryDirectory(prefix='nyxid-secret-scan-') as temporary:
        stage = Path(temporary)
        tree = stage / 'tree'
        tree.mkdir()
        paths = git(root, 'ls-tree', '-rz', '--full-tree', 'HEAD').split(b'\0')
        # One streaming object reader avoids thousands of Git processes while
        # keeping the exported bytes tied to the committed tree, not local WIP.
        with source_export.trusted_git_environment(root) as object_environment, \
                subprocess.Popen(source_export.git_command(root, 'cat-file', '--batch'), cwd=root,
                                 env=object_environment, stdin=subprocess.PIPE, stdout=subprocess.PIPE) as objects:
            for item in filter(None, paths):
                metadata, name = item.split(b'\t', 1)
                mode, kind, oid = metadata.split()
                path = normalized_path(name.decode())
                if kind != b'blob':
                    raise ValueError('secret scan cannot omit a submodule: ' + path)
                objects.stdin.write(oid + b'\n')
                objects.stdin.flush()
                header = objects.stdout.readline().split()
                if len(header) != 3 or header[:2] != [oid, b'blob']:
                    raise ValueError('committed secret-scan object is missing')
                content = objects.stdout.read(int(header[2]))
                if len(content) != int(header[2]) or objects.stdout.read(1) != b'\n':
                    raise ValueError('truncated committed secret-scan object')
                destination = tree / path
                destination.parent.mkdir(parents=True, exist_ok=True)
                # Symlink targets are scanned as text; no target is followed.
                destination.write_bytes(content)
            objects.stdin.close()
            if objects.wait() != 0:
                raise ValueError('committed secret-scan export failed')
        for scope in ('history', 'tree'):
            report = stage / (scope + '.json')
            report.touch(mode=0o600)
            command = [str(scanner), 'git' if scope == 'history' else 'dir',
                       '--config', str(root / 'security/gitleaks.toml'),
                       '--gitleaks-ignore-path', str(stage), '--ignore-gitleaks-allow',
                       '--no-banner', '--no-color', '--log-level', 'error',
                       '--report-format', 'json', '--report-path', str(report)]
            command += ['--log-opts=--all --full-history', str(root)] if scope == 'history' else [str(tree)]
            with source_export.trusted_git_environment(root) as git_environment:
                environment = {key: value for key, value in git_environment.items() if not key.startswith('GITLEAKS_')}
                # Gitleaks invokes its own git log. Preserve exact checkout
                # trust but forbid inherited config/object replacement there.
                environment['GIT_NO_REPLACE_OBJECTS'] = '1'
                result = subprocess.run(command, cwd=stage, env=environment, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            if result.returncode not in (0, 1):
                raise ValueError('secret scanner failed in ' + scope + '; exit ' + str(result.returncode))
            records = json.loads(report.read_text())
            if not isinstance(records, list) or (result.returncode == 1) != bool(records):
                raise ValueError('secret scanner report/exit mismatch')
            counts[scope] = len(records)
            for record in records:
                file = record['File']
                if scope == 'tree':
                    file = str(Path(file).relative_to(tree)) if Path(file).is_absolute() else file
                    record['File'] = file
                    content = (tree / normalized_path(file)).read_bytes()
                    commit = git(root, 'rev-parse', 'HEAD').decode().strip()
                else:
                    commit = record['Commit']
                    if not re.fullmatch(r'[0-9a-f]{40}', commit):
                        raise ValueError('invalid scanner commit')
                    content = git(root, 'show', commit + ':' + normalized_path(file))
                identity = finding_identity(record, content)
                accepted = disposition(identity, entries)
                counts['reviewed' if accepted else 'unresolved'] += 1
                findings.append(dict(scope=scope, commit=commit, line=record['StartLine'], **identity,
                                     disposition=accepted['kind'] if accepted else 'UNRESOLVED'))
    summary = dict(schema_version=1, counts=dict(counts), findings=findings)
    (output / 'secret-scan.json').write_text(json.dumps(summary, indent=2) + '\n')
    print('Full-history and committed-tree secret scan: ' + json.dumps(dict(counts), sort_keys=True))
    for item in findings:
        if item['disposition'] == 'UNRESOLVED':
            print('::error::Unresolved ' + item['rule'] + ' at ' + item['path'] + ':' + str(item['line']) +
                  ' (' + item['commit'][:12] + ', ' + item['scope'] + ')')
    return 1 if counts['unresolved'] else 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--scanner', type=Path, required=True)
    parser.add_argument('--root', type=Path, default=ROOT)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    try:
        return scan(args.root.resolve(), args.scanner.resolve(), args.output.resolve())
    except (ValueError, OSError, KeyError, subprocess.SubprocessError) as error:
        raise SystemExit('::error::Secret scan failed: ' + str(error))


if __name__ == '__main__':
    raise SystemExit(main())
