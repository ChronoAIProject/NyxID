#!/usr/bin/env python3
"""Require an actual final image scan before tagging or publishing it."""
import argparse
from pathlib import Path
import subprocess
from image_vulnerabilities import scan

if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--image', required=True)
    parser.add_argument('--component', required=True)
    parser.add_argument('--platform', choices=['linux/amd64', 'linux/arm64'], required=True)
    parser.add_argument('--commit', required=True)
    parser.add_argument('--trivy', type=Path, required=True)
    parser.add_argument('--regctl', type=Path, required=True)
    parser.add_argument('--stage', type=Path, required=True)
    args = parser.parse_args()
    try:
        code = scan(args.image, args.component, args.platform, args.commit,
                    args.trivy.resolve(), args.regctl.resolve(), args.stage.resolve())
    except (ValueError, KeyError, OSError, subprocess.SubprocessError) as error:
        raise SystemExit('::error::Final image vulnerability gate failed: ' + str(error))
    raise SystemExit(code)
