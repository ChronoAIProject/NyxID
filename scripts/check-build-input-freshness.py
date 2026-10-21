#!/usr/bin/env python3
"""Fail stale or future frozen security catalogs; use before scheduled image scans."""
import argparse
from pathlib import Path
from build_catalogs import MAX_AGE_DAYS, ca_provenance, dates, freshness

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--root', type=Path, default=Path(__file__).resolve().parents[1])
parser.add_argument('--validate-only', action='store_true', help='validate pins without checking their age')
args = parser.parse_args()
if args.validate_only:
    dates(args.root / 'security/apt-snapshots.tsv', args.root / 'security/apk')
    ca_provenance(args.root / 'security/apt-transport-ca.tsv', args.root / 'security/apt-transport-ca.pem')
    print('APT, APK and CA pin structure, provenance and byte identities passed.')
    raise SystemExit(0)
errors = freshness(args.root / 'security/apt-snapshots.tsv', args.root / 'security/apk',
                   args.root / 'security/apt-transport-ca.tsv', args.root / 'security/apt-transport-ca.pem')
if errors:
    raise SystemExit('\n'.join('::error::' + error for error in errors))
print(f'APT, APK and CA inputs are no more than {MAX_AGE_DAYS} days old; weekly candidate refresh is required.')
