#!/usr/bin/env python3
"""Admit only complete APT snapshots and inventoried Dockerfile callers."""
from pathlib import Path
import subprocess

from apt_inputs import check_apt_inputs

root = Path(__file__).resolve().parents[1]
names = subprocess.check_output(["git", "ls-files", "*Dockerfile*"], cwd=root, text=True).splitlines()
errors = check_apt_inputs(root, names)
if errors:
    raise SystemExit("\n".join(errors))
print(f"APT catalog and all {len(names)} Dockerfile definitions passed")
