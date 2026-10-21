#!/bin/sh
# Compatibility entrypoint: the historical node-only target is not acceptance
# evidence for the hosted complete Machine image.
set -eu
script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
exec sh "$script_dir/full-machine-image-smoke.sh" "$@"
