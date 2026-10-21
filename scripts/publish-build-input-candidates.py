#!/usr/bin/env python3
"""Trusted scheduled publisher: open a catalog-only candidate PR and archive inputs.

This entrypoint is never called on PRs. It never merges a PR, tags an application
release or promotes images. Its archive publisher refuses asset replacement.
"""
import argparse
from datetime import date, timedelta
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import importlib.util

from apt_inputs import ROOTS, select
from apk_archive_release import ROOT, validate_payload
from apk_inputs import ARCHES
from build_catalogs import ca_provenance, freshness, settled_snapshots
from resolve_apk_requests import REQUESTS

REPOSITORY = 'ChronoAIProject/NyxID'


def paths():
    return ['security/apt-snapshots.tsv', 'security/apt-transport-ca.tsv', 'security/apt-transport-ca.pem'] + ['security/apk/' + label + '-' + arch + '.tsv' for label in REQUESTS for arch in ARCHES]


def validate(candidate):
    if candidate.is_symlink() or any(p.is_symlink() for p in candidate.rglob('*')):
        raise ValueError('candidate must contain regular files, never symlinks')
    expected = set(paths())
    actual = {str(p.relative_to(candidate)) for p in (candidate / 'security').rglob('*') if p.is_file()}
    if actual != expected:
        raise ValueError('candidate must contain exactly the seven catalogs and CA bundle/provenance')
    select(ROOT, ROOTS, candidate / 'security/apt-snapshots.tsv')
    errors = freshness(candidate / 'security/apt-snapshots.tsv', candidate / 'security/apk',
                       candidate / 'security/apt-transport-ca.tsv', candidate / 'security/apt-transport-ca.pem')
    if errors:
        raise ValueError('\n'.join(errors))
    tag, assets = validate_payload(candidate / 'security/apk', candidate / 'payload', ARCHES)
    expected |= {'payload/' + name for name in assets}
    actual = {str(p.relative_to(candidate)) for p in candidate.rglob('*') if p.is_file()}
    directories = {str(p.relative_to(candidate)) for p in candidate.rglob('*') if p.is_dir()}
    if actual != expected or directories != {'security', 'security/apk', 'payload'}:
        raise ValueError('candidate contains unexpected files or directories')
    return tag, assets


def publish_archives(commit, payload):
    spec = importlib.util.spec_from_file_location('apk_publisher', ROOT / 'scripts/provision-apk-archives.py')
    archive_publisher = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(archive_publisher)
    archive_publisher.publish(commit, payload)


def validate_snapshot_selection(candidate, catalog_date):
    cutoff = (date.fromisoformat(catalog_date) - timedelta(days=2)).strftime('%Y%m%dT000000Z')
    expected = settled_snapshots(cutoff)
    for row in select(ROOT, ROOTS, candidate / 'security/apt-snapshots.tsv').splitlines():
        distro, _, suite, _, url, _ = row.split('\t')
        archive = 'ubuntu' if distro == 'ubuntu' else ('debian-security' if suite.endswith('-security') else 'debian')
        if url.rsplit('/', 1)[1] != expected[archive]:
            raise ValueError('APT candidate differs from the selected settled archive runs')


def publish(candidate, catalog_date):
    if (os.environ.get('GITHUB_EVENT_NAME') != 'schedule' or os.environ.get('GITHUB_REF') != 'refs/heads/main' or
            os.environ.get('GITHUB_REPOSITORY') != REPOSITORY):
        raise ValueError('candidate publication is restricted to the official main schedule')
    if date.fromisoformat(catalog_date).isoformat() != catalog_date:
        raise ValueError('non-canonical candidate date')
    tag, _ = validate(candidate)
    if tag != 'build-inputs-apk-' + catalog_date:
        raise ValueError('catalog date differs from the verified matrix')
    validate_snapshot_selection(candidate, catalog_date)
    _, checked = ca_provenance(candidate / 'security/apt-transport-ca.tsv', candidate / 'security/apt-transport-ca.pem')
    if checked.date().isoformat() != catalog_date:
        raise ValueError('CA upstream check date differs from the verified matrix')
    branch = 'automation/build-inputs-' + catalog_date
    existing = subprocess.check_output(['git', 'ls-remote', '--heads', 'origin', 'refs/heads/' + branch], cwd=ROOT, text=True).strip()
    if existing:
        commit = existing.split()[0]
        subprocess.run(['git', 'fetch', '--no-tags', 'origin', commit], cwd=ROOT, check=True)
        changed = subprocess.check_output(['git', 'diff', '--name-only', 'HEAD', commit, '--'], cwd=ROOT, text=True).splitlines()
        if set(changed) - set(paths()):
            raise ValueError('existing candidate branch changes non-catalog paths')
        for name in paths():
            if subprocess.check_output(['git', 'show', commit + ':' + name], cwd=ROOT) != (candidate / name).read_bytes():
                raise ValueError('existing daily candidate differs; refusing branch/asset replacement')
    else:
        subprocess.run(['git', 'switch', '-c', branch], cwd=ROOT, check=True)
        for name in paths():
            shutil.copyfile(candidate / name, ROOT / name)
        subprocess.run(['git', 'add', '-f', '--', *paths()], cwd=ROOT, check=True)
        change = subprocess.run(['git', 'diff', '--cached', '--quiet', '--', *paths()], cwd=ROOT)
        if change.returncode not in (0, 1):
            raise ValueError('cannot establish candidate catalog changes')
        if change.returncode == 0:
            print('Catalog candidates are already current; no branch or PR published.')
            return
        subprocess.run(['git', '-c', 'user.name=github-actions[bot]', '-c', 'user.email=41898282+github-actions[bot]@users.noreply.github.com',
                        'commit', '-m', 'chore: refresh distro build input catalogs ' + catalog_date, '--', *paths()], cwd=ROOT, check=True)
        commit = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
        subprocess.run(['git', 'push', 'origin', 'HEAD:refs/heads/' + branch], cwd=ROOT, check=True)
    publish_archives(commit, candidate / 'payload')
    prs = json.loads(subprocess.check_output(['gh', 'pr', 'list', '--repo', REPOSITORY, '--head', branch, '--state', 'all', '--json', 'url,state'], text=True))
    if not prs:
        body = f'''Refresh the four APT snapshot roots, all six APK closure catalogs, and Mozilla transport-CA provenance to {catalog_date}.

Before opening this PR, the scheduled workflow authenticated the APT snapshots on all four distro roots and both native architectures, and replayed every APK closure/tamper case on both native architectures. The CA candidate matched the official HTTPS checksum and parsed as a complete certificate bundle; APT replay consumed those exact CA bytes. Exact verified APK assets are stored under `{tag}`; product builds retain mandatory checksum, Alpine signature and signed identity verification.

Review the package and advisory changes before merging. GITHUB_TOKEN-created PRs do not trigger another PR workflow run: a maintainer must push a reviewed follow-up commit or close/reopen the PR using their own account to start required PR CI. The scheduled evidence does not replace those required checks. Only distro catalogs and the transport-CA bundle/pins are changed; language manifests, lockfiles and application sources are unchanged.
'''
        with tempfile.NamedTemporaryFile(mode='w', suffix='.md') as description:
            description.write(body)
            description.flush()
            subprocess.run(['gh', 'pr', 'create', '--repo', REPOSITORY, '--base', 'main', '--head', branch,
                            '--title', 'chore: refresh distro build input catalogs ' + catalog_date, '--body-file', description.name], check=True)
    print('Verified catalog candidate ready: ' + branch + ' at ' + commit)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--candidate', type=Path, required=True)
    parser.add_argument('--date', required=True)
    args = parser.parse_args()
    publish(args.candidate.absolute(), args.date)
