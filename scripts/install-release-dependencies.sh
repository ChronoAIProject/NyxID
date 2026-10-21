#!/bin/sh
# The reviewed OS-package contract is recorded in docs/BUILD_SECURITY.md.
# No executable package commands are accepted from the generated dist matrix.
set -eu
if [ "$(uname -s)" != Linux ]; then exit 0; fi
release_script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
. /etc/os-release
case "$ID:${VERSION_CODENAME:-}" in
    ubuntu:focal|ubuntu:noble|debian:bookworm|debian:trixie) release_root="$ID:$VERSION_CODENAME" ;;
    *) printf '%s\n' 'release APT root has no reviewed snapshot' >&2; exit 1 ;;
esac
if [ "$(id -u)" = 0 ]; then
    exec sh "$release_script_dir/apt-snapshot-install.sh" "$release_script_dir/../security/apt-snapshots.tsv" \
        "$release_root" -- clang jq pkg-config python3
fi
exec sudo -n sh "$release_script_dir/apt-snapshot-install.sh" --install-only "$release_script_dir/../security/apt-snapshots.tsv" \
    "$release_root" -- clang jq pkg-config python3
