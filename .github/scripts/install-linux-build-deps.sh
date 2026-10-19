#!/usr/bin/env bash
set -euo pipefail

build_repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
. /etc/os-release
case "$ID:${VERSION_CODENAME:-}" in
  ubuntu:focal|ubuntu:noble|debian:bookworm|debian:trixie) ;;
  *) printf '%s\n' 'CI APT root has no reviewed snapshot' >&2; exit 1 ;;
esac
sudo -n sh "$build_repo_root/scripts/apt-snapshot-install.sh" --install-only \
  "$build_repo_root/security/apt-snapshots.tsv" "$ID:$VERSION_CODENAME" -- \
  libdbus-1-dev pkg-config ripgrep
