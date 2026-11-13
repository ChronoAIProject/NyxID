#!/usr/bin/env bash
# Package the CLI with cargo-dist while retaining workspace feature unification.
set -euo pipefail
repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
export NYXID_DIST_CARGO
NYXID_DIST_CARGO=$(command -v cargo)
export CARGO="$repo/.github/scripts/dist_cargo.py"
exec dist build "$@"
