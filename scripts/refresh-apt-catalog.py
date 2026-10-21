#!/usr/bin/env python3
"""Generate HTTPS/SHA-pinned APT candidates; native signed replay must accept them."""
import argparse
from pathlib import Path
from build_catalogs import refresh_apt

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--output', type=Path, required=True)
parser.add_argument('--before', help='UTC cutoff YYYYMMDDTHHMMSSZ (default: 48 hours ago); Debian URLs use listed, settled runs at or before this time')
args = parser.parse_args()
refresh_apt(args.output, args.before)
print('APT candidate written; mandatory native signature/index/archive replay remains: ' + str(args.output))
