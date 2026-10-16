#!/bin/sh
# Compatibility entrypoint: hosted acceptance now uses the complete Machine.
set -eu
script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
exec sh "$script_dir/full-machine-image-smoke.sh" "$@"
