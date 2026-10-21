#!/usr/bin/env python3
"""Validate final standalone credential bytes and emit their subject-bound SBOM."""
import argparse
from pathlib import Path
from image_sbom import asset_document
from sbom_evidence import write

if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, required=True)
    parser.add_argument('--source-root', type=Path, required=True)
    parser.add_argument('--commit', required=True)
    parser.add_argument('--epoch', type=int, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    write(args.output, asset_document(args.root.resolve(), 'credential-accept', args.source_root.resolve(), args.commit, args.epoch))
