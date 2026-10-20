#!/usr/bin/env python3
"""Build every real Dockerfile APK prefix, without compiling application code.

These are package-layer tests, not final application image/SBOM evidence. The
exact COPY/ARG/ENV/RUN instructions through the last APK call in each stage are
retained; installs use the real Dockerfile RUN --network=none boundary.
No remote image is pushed, tagged or promoted.
"""
import argparse
from pathlib import Path
import subprocess
import tempfile

from apk_archive_layers import check_layers
from apk_inputs import prefixes as apk_prefixes

ROOT = Path(__file__).resolve().parents[1]


def prefixes(root):
    names = subprocess.check_output(["git", "ls-files", "*Dockerfile*"], cwd=root, text=True).splitlines()
    return apk_prefixes(root, names)


def run(args):
    args.output.mkdir(parents=True, exist_ok=False)
    cases = prefixes(ROOT)
    if not cases:
        raise ValueError("no Dockerfile APK stages found")
    with tempfile.TemporaryDirectory(prefix="nyxid-apk-dockerfiles-") as temporary:
        for index, (name, stage, text) in enumerate(cases):
            label = name.replace("/", "-").replace(".", "-") + "-" + stage
            dockerfile = Path(temporary) / label
            dockerfile.write_text(text)
            (args.output / (label + ".Dockerfile")).write_text(text)
            tag = "nyxid-apk-inputs:" + str(index)
            command = ["docker", "buildx", "build", "--load", "--no-cache", "--provenance=false",
                       "--platform", args.platform, "--file", str(dockerfile), "--tag", tag,
                       str(ROOT)]
            if args.pr_upstream_if_missing:
                command[-1:-1] = ['--build-arg', 'APK_INPUT_SOURCE=pr-upstream-if-missing']
            log = args.output / (label + ".log")
            with log.open("w") as output:
                result = subprocess.run(command, cwd=ROOT, stdout=output, stderr=subprocess.STDOUT, timeout=2400)
            if result.returncode:
                print(log.read_text()[-12000:], flush=True)
                raise ValueError("APK Dockerfile build failed: " + name + ":" + stage)
            saved = Path(temporary) / 'image.tar'
            subprocess.run(['docker', 'save', '--output', str(saved), tag], check=True, timeout=180)
            layer_count = check_layers(saved)
            (args.output / (label + '-layers.txt')).write_text(f'{layer_count} layers checked; no APK archive payloads\n')
            saved.unlink()
            # No network exists in this verification container. Inspect actual
            # retained distro metadata and installed inventories from the build.
            verify = r'''
set -eu
found=0
for root in /usr/share/nyxid/build-inputs/apk/*; do
  [ -d "$root" ] || continue
  found=$((found + 1))
  before=$(awk '$2=="before-sha256" {print $3}' "$root/archives.tsv")
  after=$(awk '$2=="after-sha256" {print $3}' "$root/archives.tsv")
  [ "$(sha256sum "$root/base-installed.tsv" | cut -d ' ' -f 1)" = "$before" ]
  [ "$(sha256sum "$root/installed.tsv" | cut -d ' ' -f 1)" = "$after" ]
  actual=$(awk -f /opt/nyxid-apk/apk-inventory.awk /lib/apk/db/installed | LC_ALL=C sort | sha256sum | cut -d ' ' -f 1)
  [ "$actual" = "$after" ]
done
[ "$found" -eq 1 ]
'''
            subprocess.run(["docker", "run", "--rm", "--network=none", "--user=0", "--entrypoint=sh", tag,
                            "-ec", verify, "verify", args.platform.split("/")[1]], check=True, timeout=60)
            container = subprocess.check_output(["docker", "create", "--user=0", "--entrypoint=sh", tag, "-c", "true"], text=True).strip()
            try:
                subprocess.run(["docker", "cp", container + ":/usr/share/nyxid/build-inputs",
                                str(args.output / (label + "-inputs"))], check=True, timeout=60)
            finally:
                subprocess.run(["docker", "rm", container], check=True, capture_output=True)
            print("PASS " + name + ":" + stage + " (" + args.platform + ")", flush=True)
    print(f"APK Dockerfile prefixes: {len(cases)} passed, 0 failed", flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--platform", choices=("linux/amd64", "linux/arm64"), required=True)
    parser.add_argument("--pr-upstream-if-missing", action="store_true", help="PR rehearsal only: log and allow upstream on durable HTTP 404")
    parser.add_argument("--output", type=Path, required=True)
    run(parser.parse_args())
