#!/usr/bin/env python3
"""Fetch the official Mozilla/curl bundle and matching checksum as PR candidates."""
import argparse
from datetime import datetime, timezone
from pathlib import Path
from build_catalogs import refresh_ca

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--output', type=Path, required=True)
parser.add_argument('--date', default=datetime.now(timezone.utc).date().isoformat())
args = parser.parse_args()
refresh_ca(args.output, args.date)
print('Verified CA provenance candidate written: ' + str(args.output))
