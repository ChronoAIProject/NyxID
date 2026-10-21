#!/usr/bin/env python3
"""Print a reviewed immutable remote fixture input, without pulling it."""
import argparse
import json
from pathlib import Path
import re

images = json.loads((Path(__file__).resolve().parents[1] / "security/test-images.json").read_text())["images"]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("role", choices=images)
args = parser.parse_args()
image = images[args.role]
if not re.fullmatch(r"[^\s@]+@sha256:[0-9a-f]{64}", image):
    raise SystemExit("fixture image must be immutable")
print(image)
