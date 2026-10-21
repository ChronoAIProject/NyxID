#!/usr/bin/env python3
"""Stage reviewed-format catalogs and verified release payloads from a native solver."""
import argparse
from pathlib import Path
import shutil
from apk_archive_release import stage_payload, validate_payload
from apk_inputs import ARCHES
from resolve_apk_requests import REQUESTS

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--input', type=Path, required=True)
parser.add_argument('--output', type=Path, required=True)
parser.add_argument('--arch', choices=ARCHES, required=True)
args = parser.parse_args()
catalogs = args.output / 'security/apk'
catalogs.mkdir(parents=True, exist_ok=False)
for label in REQUESTS:
    source = args.input / (label + '-' + args.arch)
    target = catalogs / (label + '-' + args.arch + '.tsv')
    shutil.copyfile(source / 'archives.tsv', target)
    stage_payload(target, args.arch, source / 'archives', args.output / 'payload')
tag, files = validate_payload(catalogs, args.output / 'payload', [args.arch])
print(f'Native candidate payload staged: {tag}, {len(files)} files ({args.arch})')
