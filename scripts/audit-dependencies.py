#!/usr/bin/env python3
"""Audit every committed dependency lock, including development dependencies.

Audits run in isolated roots with no lifecycle hooks. Reviewed mobile patches
add a clean frozen install and executable source/regression verification. Registry
errors, lock mutation, missing roots and unresolved vulnerabilities fail the gate.
"""
import argparse
from collections import Counter
import hashlib
import json
import os
from pathlib import Path
import subprocess
import shutil
import tempfile

from dependency_roots import discover
import dependency_patches
import dependency_dispositions

ROOT = Path(__file__).resolve().parents[1]


def audit(root, output, cargo_audit, npm, pnpm):
    records = dependency_patches.load_policy(root)
    dependency_patches.validate_inputs(root, records)
    risks = dependency_dispositions.load_policy(root)
    roots = discover(root)
    output.mkdir(parents=True, exist_ok=True)
    failed = []
    results = []
    environment = {key: value for key, value in os.environ.items()
                   if not key.startswith(('npm_config_', 'NPM_CONFIG_', 'CARGO_AUDIT_'))}
    environment.pop('NODE_ENV', None)
    environment.update(NPM_CONFIG_IGNORE_SCRIPTS='true', NPM_CONFIG_REGISTRY='https://registry.npmjs.org/')
    rsa_verified = dependency_dispositions.verify_rsa(root, risks, environment) if risks else False
    with tempfile.TemporaryDirectory(prefix='nyxid-dependency-audits-') as temporary:
        stage = Path(temporary).resolve()
        environment['CARGO_HOME'] = str(stage / 'cargo-home')
        environment['NPM_CONFIG_USERCONFIG'] = str(stage / 'empty.npmrc')
        (stage / 'empty.npmrc').write_text('')
        for item in roots:
            lock = root / item['lock']
            before = hashlib.sha256(lock.read_bytes()).hexdigest()
            directory = stage / str(len(results))
            directory.mkdir()
            copied_lock = directory / lock.name
            shutil.copyfile(lock, copied_lock)
            # Audit in an isolated directory: user/project .npmrc, audit.toml,
            # baselines and ignore settings cannot hide known vulnerabilities.
            for name in item['manifests']:
                if not name.endswith('package.json'):
                    continue
                package = json.loads((root / name).read_text())
                if package.get('pnpm', {}).get('auditConfig'):
                    raise ValueError('pnpm audit suppressions are not accepted: ' + name)
                destination = directory / Path(name).relative_to(lock.parent.relative_to(root))
                destination.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(root / name, destination)
            manager = item['manager']
            applicable = [r for r in records if r['lock'] == item['lock']]
            proofs = dependency_patches.install_and_verify(root, directory, applicable, pnpm, environment) if applicable else []
            tooling = [r for r in risks['dispositions'] if r['package'] == 'braces' and r['lock'] == item['lock']] if risks else []
            if tooling and not applicable: raise ValueError('tooling proof requires the verified clean frozen mobile install')
            tooling_verified = all(dependency_dispositions.verify_tooling(root, directory, r, environment) for r in tooling) if tooling else False
            command = ([str(cargo_audit), 'audit', '--file', str(copied_lock), '--json'] if manager == 'cargo' else
                       [str(npm if manager == 'npm' else pnpm), 'audit', '--json', '--audit-level=low'])
            result = subprocess.run(command, cwd=directory, env=environment, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            try:
                report = json.loads(result.stdout)
            except (ValueError, UnicodeDecodeError) as error:
                raise ValueError('invalid audit report: ' + item['lock']) from error
            if result.returncode not in (0, 1):
                raise ValueError('audit command failed: ' + item['lock'] + ', exit ' + str(result.returncode))
            if hashlib.sha256(lock.read_bytes()).hexdigest() != before or \
                    hashlib.sha256(copied_lock.read_bytes()).hexdigest() != before:
                raise ValueError('audit mutated dependency lock: ' + item['lock'])
            if manager == 'cargo':
                section = report.get('vulnerabilities')
                if not isinstance(section, dict) or not isinstance(section.get('list'), list):
                    raise ValueError('missing Cargo vulnerability results')
                count = len(section['list'])
                if section.get('count') != count or section.get('found') != bool(count):
                    raise ValueError('Cargo audit report count mismatch')
                errors = bool(report.get('error'))
            else:
                counts = report.get('metadata', {}).get('vulnerabilities')
                if not isinstance(counts, dict) or not all(type(v) is int and v >= 0 for v in counts.values()):
                    raise ValueError('missing npm/pnpm vulnerability results')
                count = sum(counts.get(severity, 0) for severity in ('info', 'low', 'moderate', 'high', 'critical'))
                errors = bool(report.get('error'))
            if errors or result.returncode != (1 if count else 0):
                raise ValueError('audit API/error/exit mismatch: ' + item['lock'])
            name = item['lock'].replace('/', '__') + '.audit.json'
            (output / name).write_text(json.dumps(report, indent=2) + '\n')
            reviewed = dependency_patches.classify(report, item['lock'], applicable, proofs)
            if manager == 'cargo':
                reviewed += dependency_dispositions.classify_cargo(report, item['lock'], risks, rsa_verified)
            elif tooling:
                reviewed += dependency_dispositions.classify_tooling(report, item['lock'], risks, tooling_verified)
            if manager == 'pnpm' and reviewed and len(report.get('advisories', {})) != count:
                raise ValueError('pnpm advisory/count mismatch')
            unresolved = count - len(reviewed)
            if unresolved < 0: raise ValueError('reviewed finding count exceeds raw audit count')
            item = dict(item, vulnerabilities=count, reviewed_dispositions=reviewed, unresolved=unresolved,
                        lock_sha256=before, status='failed' if unresolved else 'passed')
            results.append(item)
            print(f"{item['lock']}: {count} registry findings, {len(reviewed)} exact reviewed, {unresolved} unresolved; {len(item['manifests'])} manifest(s)")
            if unresolved:
                failed.append(item['lock'])
        if risks: dependency_dispositions.validate_rsa_surface(root, risks)
        (output / 'dependency-audits.json').write_text(json.dumps(dict(roots=results), indent=2) + '\n')
        print('Audited roots: ' + json.dumps(dict(Counter(item['manager'] for item in roots))))
        if failed:
            print('::error::Dependency vulnerabilities: ' + ', '.join(failed))
        return 1 if failed else 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, default=ROOT)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--cargo-audit', type=Path, required=True)
    parser.add_argument('--npm', type=Path, required=True)
    parser.add_argument('--pnpm', type=Path, required=True)
    args = parser.parse_args()
    try:
        return audit(args.root.resolve(), args.output.resolve(), args.cargo_audit.resolve(), args.npm.resolve(), args.pnpm.resolve())
    except (ValueError, OSError, KeyError, subprocess.SubprocessError) as error:
        raise SystemExit('::error::Dependency audit failed: ' + str(error))


if __name__ == '__main__':
    raise SystemExit(main())
