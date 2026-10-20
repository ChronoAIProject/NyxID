#!/usr/bin/env python3
"""Generate and validate SBOMs from the exact staged final image; never publish."""
import argparse
from pathlib import Path
from image_sbom import generate

if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--stage', type=Path, required=True)
    parser.add_argument('--source-root', type=Path, required=True)
    parser.add_argument('--epoch', type=int, required=True)
    parser.add_argument('--extractor', type=Path, required=True)
    parser.add_argument('--syft', type=Path, required=True)
    args = parser.parse_args()
    generate(args.stage.resolve(), args.source_root.resolve(), args.epoch, args.extractor.resolve(), args.syft.resolve())
