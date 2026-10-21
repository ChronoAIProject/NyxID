#!/usr/bin/env python3
"""Fail when the current catalog release/assets are absent or incomplete."""
import os
from pathlib import Path

from apk_durability import check, report

root = Path(__file__).resolve().parents[1]
try:
    errors = check(root / 'security/apk')
except Exception as error:
    raise SystemExit('::error::APK durability check could not complete: ' + str(error))
message = '\n'.join(errors) if errors else 'APK durable archives present: every current catalog and archive has a matching uploaded release asset.'
if os.environ.get('GITHUB_STEP_SUMMARY'):
    with Path(os.environ['GITHUB_STEP_SUMMARY']).open('a') as summary:
        summary.write('APK durable archives present\n\n' + message + '\n')
if errors:
    raise SystemExit(report(errors, os.environ.get('GITHUB_EVENT_NAME', 'local')))
print(message)
