#!/usr/bin/env python3
"""Build every real Dockerfile APT prefix, without compiling application code.

These are package-layer tests, not final application image/SBOM evidence. The
exact COPY/ARG/ENV/RUN instructions through the last APT call in each stage are
retained. Test-only MACHINE_IMAGE consumes the machine desktop prefix built in
this same run. No remote image is pushed, tagged or promoted.
"""
import argparse
from pathlib import Path
import subprocess
import tempfile

from apt_inputs import INSTALLER, projection, stages

ROOT = Path(__file__).resolve().parents[1]


def prefixes(root):
    names = subprocess.check_output(["git", "ls-files", "*Dockerfile*"], cwd=root, text=True).splitlines()
    result = []
    for name in names:
        parsed = stages((root / name).read_text())
        for stage in parsed:
            if any(INSTALLER in raw for raw in stage.body):
                result.append((name, stage.name, projection(stage, parsed)))
    # The derived test-image prefixes inherit this same build's package bytes.
    result.sort(key=lambda item: (item[:2] != ("cli/Dockerfile.machine", "desktop"), item[:2]))
    return result


def run(args):
    args.output.mkdir(parents=True, exist_ok=False)
    cases = prefixes(ROOT)
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
            command = ["docker", "buildx", "build", "--load", "--no-cache", "--provenance=false",
                       "--platform", args.platform, "--file", str(dockerfile), "--tag", tag,
                       "--build-arg", "MACHINE_IMAGE=nyxid-apt-machine:ci", str(ROOT)]
            log = args.output / (label + ".log")
            with log.open("w") as output:
                result = subprocess.run(command, cwd=ROOT, stdout=output, stderr=subprocess.STDOUT, timeout=2400)
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
