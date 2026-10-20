#!/usr/bin/env python3
"""Build and verify reviewed local images before applying the requested tag.

Dirty checkouts fail before Docker runs. Native Cargo builds remain available
for investigation and retain their git-unverified/git-dirty identity.
"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[1]


def module(name, filename):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(filename))
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


source = module("image_source", "export-build-source.py")
runner = module("image_runner", "run-exported-build.py")

# The wrapper admits reviewed final application targets and one existing test
# target. An unrecognized stage must never turn into a verification bypass.
SHIPPED = {
    'backend/Dockerfile': {'': 'backend'},
    'frontend/Dockerfile': {'': 'frontend', 'runtime': 'frontend'},
    'cli/Dockerfile.node': {'': 'node-agent'},
    'cli/Dockerfile.machine': {'': 'nyxid-node-machine', 'desktop': 'nyxid-node-machine'},
    'cli/Dockerfile.machine-node-only': {'': 'machine-node-only'},
    'cli/Dockerfile.machine-hosted': {'': 'machine-hosted', 'hosted': 'machine-hosted',
                                      'desktop': 'machine-hosted-desktop'},
    'cli/Dockerfile.machine-updater': {'': 'nyxid-machine-updater', 'final': 'nyxid-machine-updater',
                                       'production': 'nyxid-machine-updater'},
    'Dockerfile.glama': {'': 'mcp-demo'},
}
TEST_TARGETS = {('cli/Dockerfile.machine-updater', 'e2e')}


def component_for(dockerfile, target):
    selected = target or ''
    if (dockerfile, selected) in TEST_TARGETS:
        return None
    if selected not in SHIPPED.get(dockerfile, {}):
        raise ValueError('unreviewed image/stage: no mandatory final identity policy')
    return SHIPPED[dockerfile][selected]


def native_platform():
    value = subprocess.check_output(['docker', 'version', '--format', '{{.Server.Os}}/{{.Server.Arch}}'],
                                    text=True).strip()
    if value not in ('linux/amd64', 'linux/arm64'):
        raise ValueError('reviewed application images require Linux amd64 or arm64')
    return value


def check_identity(tree, destination, image_id, component, platform, exported, result):
    checker = tree / 'scripts/check-image-identity.py'
    if checker.is_symlink() or not checker.is_file():
        raise ValueError('mandatory final image identity checker is missing or not regular')
    # Build the inspector from the same authenticated export. No caller-supplied
    # executable, alternate checker or opt-out is accepted.
    manifest = tree / 'security/artifact-tools/Cargo.toml'
    if manifest.is_symlink() or not manifest.is_file():
        raise ValueError('mandatory final executable inspector source is missing')
    artifact_target = destination / 'artifact-target'
    environment = {key: value for key, value in os.environ.items() if not key.startswith('GIT_')}
    environment.update(CARGO_BUILD_JOBS='2', CARGO_TARGET_DIR=str(artifact_target),
                       NYXID_GIT_HASH=exported['source']['commit'],
                       SOURCE_COMMIT=exported['source']['commit'],
                       SOURCE_DATE_EPOCH=str(exported['source']['epoch']), PYTHONDONTWRITEBYTECODE='1')
    command = ['cargo', 'build', '--locked', '--release', '--manifest-path', str(manifest)]
    result['inspector_build_exit_code'] = subprocess.run(command, cwd=tree, env=environment).returncode
    if result['inspector_build_exit_code'] != 0:
        raise ValueError('mandatory final executable inspector build failed')
    extractor = artifact_target / 'release/nyxid-artifact-info'
    if extractor.is_symlink() or not extractor.is_file():
        raise ValueError('mandatory final executable inspector is missing')
    result['command'] = [sys.executable, str(checker), '--image', image_id, '--component', component,
                         '--arch', platform.split('/')[1], '--source-root', str(tree),
                         '--commit', exported['source']['commit'], '--epoch', str(exported['source']['epoch']),
                         '--extractor', str(extractor)]
    result['exit_code'] = subprocess.run(result['command'], cwd=tree, env=environment).returncode
    if result['exit_code'] != 0:
        raise ValueError('mandatory final image identity check failed')


def build(root, destination, dockerfile, tag, target=None, platform=None, contexts=()):
    commit = source.git(root, "rev-parse", "HEAD").decode().strip()
    relative = str(source.relative(dockerfile))
    component = component_for(relative, target)
    if platform is not None and platform not in ('linux/amd64', 'linux/arm64'):
        raise ValueError('one reviewed Linux platform is required')
    exported = source.export(root, destination, commit)
    tree = destination / "source"
    file = tree / relative
    if file.is_symlink() or not file.is_file():
        raise ValueError("Dockerfile must be a regular committed source input")
    platform = platform or native_platform()
    iidfile = destination / 'image-id'
    command = ["docker", "build", "--load", "--build-arg", "NYXID_GIT_HASH=" + commit,
               "--build-arg", "SOURCE_DATE_EPOCH=" + str(exported["source"]["epoch"]),
               "--file", str(file), "--iidfile", str(iidfile)]
    if target:
        command += ["--target", target]
    command += ["--platform", platform]
    for context in contexts:
        command += ["--build-context", context]
    command.append(str(tree))
    receipt_path = destination / 'image-build-execution.json'
    code = runner.run(destination, commit, receipt_path, command)
    receipt = json.loads(receipt_path.read_text())
    receipt['build_exit_code'] = code
    receipt['image'] = dict(id=None, component=component, platform=platform, requested_tag=tag, tagged=False)
    check = receipt['identity_check'] = dict(status='not-run', exit_code=None)
    try:
        if code == 0:
            if iidfile.is_symlink() or not iidfile.is_file() or iidfile.stat().st_size > 128:
                raise ValueError('build did not produce a bounded regular immutable image ID')
            image_id = iidfile.read_text().strip()
            if not re.fullmatch(r'sha256:[0-9a-f]{64}', image_id):
                raise ValueError('build did not produce one immutable image ID')
            receipt['image']['id'] = image_id
            if component:
                check['status'] = 'failed'
                check_identity(tree, destination, image_id, component, platform, exported, check)
            try:
                source.verify(destination, commit)
            except (ValueError, OSError) as error:
                receipt['post_build_source_verified'] = False
                receipt['post_build_source_error'] = str(error)
                raise
            check['status'] = 'passed' if component else 'not-applicable-test-target'
            # No tag is applied until the exact final image passes. Failure
            # preserves any pre-existing tag instead of replacing it with bad bytes.
            code = subprocess.run(['docker', 'tag', '--', image_id, tag], check=False).returncode
            receipt['image']['tagged'] = code == 0
    except (ValueError, OSError, subprocess.SubprocessError) as error:
        check['error'] = str(error)
        print(str(error), file=sys.stderr)
        code = 1
    finally:
        receipt['exit_code'] = code if code >= 0 else 128 - code
        temporary = receipt_path.with_suffix('.tmp')
        temporary.write_text(json.dumps(receipt, indent=2) + '\n')
        temporary.replace(receipt_path)
    code = receipt['exit_code']
    return code, exported


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=ROOT)
    parser.add_argument("--file", required=True, help="committed Dockerfile relative to repository root")
    parser.add_argument("--tag", required=True)
    parser.add_argument("--target")
    parser.add_argument("--platform")
    parser.add_argument("--build-context", action="append", default=[],
                        help="additional input; its identity requires separate verification")
    parser.add_argument("--destination", type=Path, help="new path outside Git for retained inputs/receipts")
    args = parser.parse_args()
    destination = args.destination or Path(tempfile.mkdtemp(prefix="nyxid-local-image-")) / "export"
    code, receipt = build(args.root.resolve(), destination.resolve(), args.file,
                          args.tag, args.target, args.platform, args.build_context)
    print(json.dumps({"source": receipt["source"], "archive": receipt["archive"],
                      "evidence": str(destination), "exit_code": code}))
    raise SystemExit(code)


if __name__ == "__main__":
    main()
