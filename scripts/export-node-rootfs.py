#!/usr/bin/env python3
"""Export only declared node runtime files from the snapshot-installed builder.

This never resolves or downloads packages: apt-snapshot-install.sh has already
verified and installed their bytes. The final scratch image retains selected
package control records and file lists so OS scanners inventory what ships.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import subprocess

PACKAGES = ('base-files', 'libc6', 'libgcc-s1', 'netbase', 'tzdata')
SLIM_METADATA = ('/usr/share/doc/', '/usr/share/man/', '/usr/share/lintian/', '/usr/share/locale/')


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def export(destination, source=Path('/'), query=subprocess.check_output, component='node-agent'):
    if component not in ('node-agent', 'backend'):
        raise ValueError('undeclared minimal rootfs component')
    if destination.exists():
        raise ValueError('rootfs destination must not already exist')
    # Query exact installed binary package identities; never parse a source
    # package name as an installed package or import all builder dependencies.
    records = []
    for package in PACKAGES:
        control = query(['dpkg-query', '-s', package], text=True)
        fields = dict(line.split(': ', 1) for line in control.splitlines() if ': ' in line and not line.startswith(' '))
        if fields.get('Package') != package or fields.get('Status') != 'install ok installed':
            raise ValueError('required installed runtime package missing: ' + package)
        records.append((package, fields, control))
    destination.mkdir(parents=True)
    # Match Debian's merged-/usr layout. Copying into /lib must resolve inside
    # this root, never through absolute host symlinks.
    for name, target in (('bin', 'usr/bin'), ('sbin', 'usr/sbin'), ('lib', 'usr/lib'), ('lib64', 'usr/lib64')):
        (destination / target).mkdir(parents=True, exist_ok=True)
        (destination / name).symlink_to(target)
    info = destination / 'var/lib/dpkg/info'
    info.mkdir(parents=True)
    manifests = []
    for package, fields, control in records:
        paths = query(['dpkg-query', '-L', package], text=True).splitlines()
        retained, omitted_metadata = [], []
        for name in paths:
            if name in ('/.', '/'):
                continue
            parts = PurePosixPath(name).parts
            if not name.startswith('/') or '..' in parts or '\\' in name:
                raise ValueError('unsafe package path')
            original = source / name.lstrip('/')
            target = destination / name.lstrip('/')
            if name in ('/bin', '/sbin', '/lib', '/lib64'):
                continue
            target.parent.mkdir(parents=True, exist_ok=True)
            if not target.parent.resolve().is_relative_to(destination.resolve()):
                raise ValueError('package path escapes exported root')
            if original.is_symlink():
                link = original.readlink()
                # Package links may be absolute in a Debian root. Rewrite the
                # common merged-/usr link to a relative in-root target.
                if target.is_symlink():
                    if target.readlink() != link: raise ValueError('conflicting package symlink')
                elif not target.exists():
                    target.symlink_to(link)
                else:
                    raise ValueError('conflicting package path')
            elif original.is_dir():
                target.mkdir(exist_ok=True)
            elif original.is_file():
                if target.exists() and digest(target) != digest(original):
                    raise ValueError('conflicting package file')
                shutil.copy2(original, target, follow_symlinks=False)
            elif not original.exists() and name.startswith(SLIM_METADATA):
                # Official slim roots intentionally exclude these dpkg paths.
                # Keep the final .list honest about the actual copied bytes.
                omitted_metadata.append(name)
                continue
            else:
                raise ValueError('package file missing or special: ' + name)
            retained.append(name)
        identity = package + (':' + fields['Architecture'] if fields.get('Multi-Arch') == 'same' else '')
        (info / (identity + '.list')).write_text('\n'.join(retained) + '\n')
        manifests.append(dict(package=package, version=fields['Version'], architecture=fields['Architecture'],
                              omitted_metadata=omitted_metadata))
    (destination / 'var/lib/dpkg/status').write_text('\n\n'.join(control.strip() for _, _, control in records) + '\n')
    ca = source / 'etc/ssl/certs/ca-certificates.crt'
    ca_control = query(['dpkg-query', '-s', 'ca-certificates'], text=True)
    if not ca.is_file() or 'Status: install ok installed\n' not in ca_control:
        raise ValueError('snapshot CA bundle missing')
    ca_target = destination / 'etc/ssl/certs/ca-certificates.crt'
    ca_target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(ca, ca_target)
    ca_target.chmod(0o644)
    evidence = destination / 'usr/share/nyxid/build-inputs'
    evidence.mkdir(parents=True, exist_ok=True)
    shutil.copytree(source / 'usr/share/nyxid/build-inputs/apt', evidence / 'apt')
    (evidence / 'node-rootfs.json').write_text(json.dumps(dict(schema='nyxid.node-rootfs.v1', component=component,
        packages=manifests, ca_package_control=ca_control, ca_sha256=digest(ca_target)), indent=2) + '\n')
    if component == 'backend':
        # Preserve the Debian image's system nyxid UID/GID without shipping
        # useradd, a shell, or any of their runtime dependency closure.
        (destination / 'etc/passwd').write_text('nyxid:x:999:999::/app:/sbin/nologin\n')
        (destination / 'etc/group').write_text('nyxid:x:999:\n')
        for directory in ('app/keys', 'var/lib/nyxid/tuf'):
            target = destination / directory
            target.mkdir(parents=True)
            os.chown(target, 999, 999)
        (destination / 'var/lib/nyxid/tuf').chmod(0o700)
    else:
        (destination / 'app/config').mkdir(parents=True)
    print('Exported ' + component + ' runtime: ' + ', '.join(PACKAGES) + '; snapshot CA bundle; retained dpkg inventory')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--destination', type=Path, required=True)
    parser.add_argument('--component', choices=('node-agent', 'backend'), default='node-agent')
    args = parser.parse_args()
    export(args.destination.resolve(), component=args.component)
