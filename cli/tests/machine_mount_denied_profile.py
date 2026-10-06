#!/usr/bin/env python3
"""Emulate docker-default's AppArmor mount denial on hosts without AppArmor."""
import json
import sys
from pathlib import Path

source, destination = map(Path, sys.argv[1:])
profile = json.loads(source.read_text())
denied = {"mount", "umount2", "pivot_root"}
for rule in profile["syscalls"]:
    rule["names"] = [name for name in rule["names"] if name not in denied]
profile["syscalls"] = [rule for rule in profile["syscalls"] if rule["names"]]
profile["syscalls"].append(
    {"names": sorted(denied), "action": "SCMP_ACT_ERRNO", "errnoRet": 1}
)
destination.write_text(json.dumps(profile, indent=2) + "\n")
