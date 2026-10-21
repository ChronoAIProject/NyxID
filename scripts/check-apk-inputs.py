#!/usr/bin/env python3
"""Fail closed on mutable APK callers or incomplete reviewed catalogs."""
from pathlib import Path
import subprocess
from apk_inputs import check_apk_inputs

root = Path(__file__).resolve().parents[1]
names = subprocess.check_output(["git", "ls-files", "*Dockerfile*"], cwd=root, text=True).splitlines()
errors = check_apk_inputs(root, names)
if errors:
    raise SystemExit("\n".join(errors))
print("APK inputs: all catalogs and Dockerfile callers passed")
