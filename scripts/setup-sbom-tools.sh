#!/bin/sh
# Run on the native inspection runner (Python >=3.9); never inside a Focal builder.
set -eu
sbom_setup_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
sbom_setup_destination=$1
python3 -m venv "$sbom_setup_destination/python"
"$sbom_setup_destination/python/bin/python" -m pip install --require-hashes --only-binary=:all: -r "$sbom_setup_root/security/python-sbom-requirements.txt"
python3 "$sbom_setup_root/scripts/bootstrap-build-tool.py" syft --bin-dir "$sbom_setup_destination/bin"
