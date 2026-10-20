#!/usr/bin/env python3
"""Copy the exact passing staged OCI subject by digest; never rebuild or retag."""
import argparse
import os
from pathlib import Path
import re
import subprocess
from image_vulnerabilities import verify_stage
from image_sbom import verify as verify_sbom
from publish_sbom import attach


def publish(stage, repository, regctl):
    if not re.fullmatch(r'ghcr.io/[a-z0-9_-]+/nyxid/[a-z0-9_-]+', repository):
        raise ValueError('reviewed GHCR application repository required')
    receipt = verify_stage(stage)
    verify_sbom(stage)
    digest = receipt['subject']['digest']
    source = 'ocidir://' + str(stage / 'oci') + '@' + digest
    destination = repository + '@' + digest
    subprocess.run([str(regctl), 'image', 'copy', source, destination], check=True)
    actual = subprocess.check_output([str(regctl), 'manifest', 'digest', destination], text=True).strip()
    if actual != digest:
        raise ValueError('registry digest differs from scanned publication subject')
    attach(regctl, destination, stage / 'sbom/image.cdx.json')
    return digest


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--stage', type=Path, required=True)
    parser.add_argument('--repository', required=True)
    parser.add_argument('--regctl', type=Path, required=True)
    args = parser.parse_args()
    try:
        digest = publish(args.stage.resolve(), args.repository, args.regctl.resolve())
        with open(os.environ['GITHUB_OUTPUT'], 'a') as output:
            output.write('digest=' + digest + '\n')
        print('Published scanned subject ' + digest)
    except (ValueError, KeyError, OSError, subprocess.SubprocessError) as error:
        raise SystemExit('::error::Scanned image publication failed: ' + str(error))
