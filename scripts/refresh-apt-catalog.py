#!/usr/bin/env python3
"""Generate HTTPS/SHA-pinned APT candidates; native signed replay must accept them."""
import argparse
from datetime import datetime, timedelta, timezone
from pathlib import Path
from build_catalogs import refresh_apt

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--output', type=Path, required=True)
parser.add_argument('--stamp', default=(datetime.now(timezone.utc) - timedelta(days=1)).strftime('%Y%m%dT000000Z'))
args = parser.parse_args()
refresh_apt(args.output, args.stamp)
print('APT candidate written; mandatory native signature/index/archive replay remains: ' + str(args.output))
