#!/usr/bin/env bash
set -euo pipefail

build_repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
# Main before the RustCrypto keyring migration still needs native DBus, even
# when measured by a PR whose own CLI no longer links it. This reviewed addition
# is restricted to the base-coverage caller and uses the same signed snapshot.
build_packages=(pkg-config ripgrep)
case "$#:${1:-}" in
  0:) ;;
  1:--base-coverage) build_packages+=(libdbus-1-dev) ;;
  *) printf '%s\n' 'unsupported build dependency mode' >&2; exit 1 ;;
esac
. /etc/os-release
case "$ID:${VERSION_CODENAME:-}" in
  ubuntu:focal|ubuntu:noble|debian:bookworm|debian:trixie) ;;
  *) printf '%s\n' 'CI APT root has no reviewed snapshot' >&2; exit 1 ;;
esac
sudo -n sh "$build_repo_root/scripts/apt-snapshot-install.sh" --install-only \
  "$build_repo_root/security/apt-snapshots.tsv" "$ID:$VERSION_CODENAME" -- \
  "${build_packages[@]}"
