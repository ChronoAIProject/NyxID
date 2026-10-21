#!/usr/bin/env python3
"""Build every real Dockerfile APT prefix, without compiling application code.

These are package-layer tests, not final application image/SBOM evidence. The
exact COPY/ARG/ENV/RUN instructions through the last APT call in each stage are
retained. Test-only MACHINE_IMAGE consumes an in-build machine desktop prefix;
the isolated BuildKit builder cannot see Docker engine-local tags. No remote
image is pushed, tagged or promoted.
"""
import argparse
from pathlib import Path
import subprocess
import tempfile
import time

from apt_inputs import INSTALLER, projection, stages

ROOT = Path(__file__).resolve().parents[1]
# 40 minutes bounds a cold desktop closure (hundreds of archives), package
# configuration and image loading. Each prefix logs elapsed time and failures.
# Reuse only this job's fresh BuildKit builder, never imported external caches.
BUILD_TIMEOUT = 2400


def build_command(dockerfile, tag, platform, root):
    return ['docker', 'buildx', 'build', '--load', '--progress=plain', '--provenance=false',
            '--platform', platform, '--file', str(dockerfile), '--tag', tag,
            '--build-arg', 'MACHINE_IMAGE=desktop', str(root)]


def prefixes(root):
    names = subprocess.check_output(["git", "ls-files", "*Dockerfile*"], cwd=root, text=True).splitlines()
    result = []
    for name in names:
        parsed = stages((root / name).read_text())
        for stage in parsed:
            if any(INSTALLER in raw for raw in stage.body):
                result.append((name, stage.name, projection(stage, parsed)))
    desktop = next(text for name, stage, text in result if (name, stage) == ('cli/Dockerfile.machine', 'desktop'))
    for index, (name, stage, text) in enumerate(result):
        parsed = stages(text)
        if parsed[0].base == '${MACHINE_IMAGE}':
            result[index] = (name, stage, ''.join(parsed[0].arguments) + desktop + ''.join(parsed[0].body))
    # Derived test images inherit the exact desktop package instructions above.
    result.sort(key=lambda item: (item[:2] != ("cli/Dockerfile.machine", "desktop"), item[:2]))
    return result


def minimal_ubuntu(root):
    # Exercise real server HTTP->HTTPS redirects without the host proxy or an
    # inherited CA package. Transport trust must remain outside the system store.
    base = next(s.base for s in stages((root / 'cli/Dockerfile.machine').read_text()) if s.name == 'desktop')
    from apt_inputs import INPUTS
    return 'FROM ' + base + ' AS runtime\nCOPY ' + ' '.join(INPUTS) + ' /opt/nyxid-apt/\n' + \
        'RUN chmod 0700 /opt/nyxid-apt && test ! -s /etc/ssl/certs/ca-certificates.crt && ' + INSTALLER + \
        ' ubuntu:noble -- libdbus-1-dev && test ! -s /etc/ssl/certs/ca-certificates.crt\n'


def run(args):
    args.output.mkdir(parents=True, exist_ok=False)
    cases = [("minimal-ubuntu-transport", "runtime", minimal_ubuntu(ROOT))] + prefixes(ROOT)
    if not cases:
        raise ValueError("no Dockerfile APT stages found")
    with tempfile.TemporaryDirectory(prefix="nyxid-apt-dockerfiles-") as temporary:
        for index, (name, stage, text) in enumerate(cases):
            label = name.replace("/", "-").replace(".", "-") + "-" + stage
            dockerfile = Path(temporary) / label
            dockerfile.write_text(text)
            (args.output / (label + ".Dockerfile")).write_text(text)
            tag = "nyxid-apt-inputs:" + str(index)
            if (name, stage) == ("cli/Dockerfile.machine", "desktop"):
                tag = "nyxid-apt-machine:ci"
            # Derived test prefixes contain the identical desktop prefix in
            # the same builder. Replaying it with --no-cache redownloads and
            # configures the entire desktop/Chromium closure for each case.
            command = build_command(dockerfile, tag, args.platform, ROOT)
            log = args.output / (label + ".log")
            started = time.monotonic()
            try:
                with log.open("w") as output:
                    result = subprocess.run(command, cwd=ROOT, stdout=output, stderr=subprocess.STDOUT, timeout=BUILD_TIMEOUT)
            except subprocess.TimeoutExpired as error:
                print(log.read_text()[-16000:], flush=True)
                raise ValueError('APT package-prefix build timed out: ' + label) from error
            finally:
                print(f'APT prefix {label}: {time.monotonic() - started:.1f}s (bound {BUILD_TIMEOUT}s)', flush=True)
            if result.returncode:
                print(log.read_text()[-12000:], flush=True)
                raise ValueError("APT Dockerfile build failed: " + name + ":" + stage)
            # No network exists in this verification container. Inspect actual
            # retained distro metadata and installed inventories from the build.
            verify = r'''
set -eu
[ "$(dpkg --print-architecture)" = "$1" ]
found=0
for root in /usr/share/nyxid/build-inputs/apt/*; do
  [ -d "$root" ] || continue
  found=$((found + 1))
  [ "$(cat "$root/architecture")" = "$1" ]
  [ "$(cat "$root/schema")" = nyxid.apt-snapshot-install.v1 ]
  test -s "$root/installed-packages.tsv"
  test -s "$root/snapshots.tsv"
  sha256sum -c "$root/metadata-sha256.txt"
done
[ "$found" -gt 0 ]
'''
            subprocess.run(["docker", "run", "--rm", "--network=none", "--entrypoint=sh", tag,
                            "-ec", verify, "verify", args.platform.split("/")[1]], check=True, timeout=60)
            container = subprocess.check_output(["docker", "create", "--entrypoint=sh", tag, "-c", "true"], text=True).strip()
            try:
                subprocess.run(["docker", "cp", container + ":/usr/share/nyxid/build-inputs",
                                str(args.output / (label + "-inputs"))], check=True, timeout=60)
            finally:
                subprocess.run(["docker", "rm", container], check=True, capture_output=True)
            print("PASS " + name + ":" + stage + " (" + args.platform + ")", flush=True)
    print(f"APT Dockerfile prefixes: {len(cases)} passed, 0 failed", flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--platform", choices=("linux/amd64", "linux/arm64"), required=True)
    parser.add_argument("--output", type=Path, required=True)
    run(parser.parse_args())
