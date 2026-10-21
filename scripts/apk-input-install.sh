#!/bin/sh
# Download authenticated reviewed archives, then replay with RUN --network=none.
set -eu
export LC_ALL=C
fail() { printf '%s\n' "$*" >&2; exit 1; }
[ "$#" -ge 4 ] && [ "$#" -le 5 ] && [ "$(id -u)" = 0 ] || fail 'usage: apk-input-install.sh --download|--install CATALOG BASE-IMAGE ARCHIVE-DIRECTORY [SOURCE-MODE] (root)'
mode=$1 catalog=$2 base=$3 archives=$4
source_mode=${5:-prefer-durable}
case "$source_mode" in prefer-durable|provision-upstream) ;; *) fail 'invalid APK source mode' ;; esac
[ "$mode" = --download ] || [ "$#" = 4 ] || fail 'source mode is download-only'
case "${archives}" in /*/archives) ;; *) fail 'APK archive path must be an absolute archives directory' ;; esac
[ ! -L "${archives}" ] || fail 'APK archive directory must not be symlinked'
case "$mode" in --download|--install) ;; *) fail 'invalid APK installer mode' ;; esac
[ -f "$catalog" ] || fail 'APK catalog missing'
script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
architecture=$(apk --print-arch)
# Validate the entire catalog before creating output or making network requests.
rows=$(awk -v base="$base" -v architecture="$architecture" -f "$script_dir/apk-catalog.awk" "$catalog") || fail 'invalid APK catalog'
field() { awk -v key="$1" '$1=="#" && $2==key {print $3}' "$catalog"; }
inventory() { awk -f "$script_dir/apk-inventory.awk" "${1:-/lib/apk/db/installed}" | LC_ALL=C sort; }
before=$(inventory | sha256sum | cut -d ' ' -f 1)
[ "$before" = "$(field before-sha256)" ] || fail 'APK base inventory mismatch'
if [ "$mode" = --download ]; then
    [ ! -e "${archives}" ] || fail 'APK archive directory must be fresh'
    mkdir -p "${archives}"
else
    [ -d "${archives}" ] || fail 'APK archives missing'
fi
set --
tab=$(printf '\t')
while IFS="$tab" read -r name version arch sha durable upstream; do
    archive=${archives}/$name-$version.apk
    if [ "$mode" = --download ]; then
        sh "$script_dir/apk-input-fetch.sh" "$durable" "$upstream" "$archive" "$source_mode" "$sha"
    fi
    [ -f "$archive" ] && [ ! -L "$archive" ] || fail 'APK archive missing or symlinked'
    printf '%s  %s\n' "$sha" "$archive" | sha256sum -c - >/dev/null || fail 'APK archive checksum mismatch'
    apk verify "$archive" || fail 'APK archive signature verification failed'
    metadata=$(tar -xzOf "$archive" .PKGINFO)
    printf '%s\n' "$metadata" | awk -F ' = ' -v name="$name" -v version="$version" -v arch="$arch" '
      $1=="pkgname" {if($2!=name) exit 1; names++}
      $1=="pkgver" {if($2!=version) exit 1; versions++}
      $1=="arch" {if($2!=arch) exit 1; arches++}
      END {if(names!=1 || versions!=1 || arches!=1) exit 1}' || fail 'APK signed identity mismatch'
    set -- "$@" "$archive"
done <<EOF_ROWS
$rows
EOF_ROWS
[ "$mode" = --install ] || exit 0
[ "$#" -gt 0 ] || fail 'empty APK archive selection'
work=$(mktemp -d /tmp/nyxid-apk-install.XXXXXXXX)
trap 'rm -rf -- "$work"' EXIT HUP INT TERM
# Ignore inherited indexes/cache/repositories: the complete closure consists
# only of the pinned base plus these explicit, signature-checked archives.
# A successful --simulate is insufficient: an installed virtual provider may
# satisfy a missing archive while producing a different final inventory. Replay
# the actual transaction into an isolated copy of the package database first.
# It has no mutable repositories/cache and executes no scripts or triggers.
# This root contains only package state/keyrings, never the live filesystem.
# APK 2 opens its static-cache parent even with --no-cache. Supply the empty
# layout in scratch; no live cache, indexes or network are made available.
mkdir -p "$work/root/lib" "$work/root/etc" "$work/root/var/cache/apk"
cp -R /lib/apk "$work/root/lib/apk"
cp -R /etc/apk "$work/root/etc/apk"
apk --root "$work/root" --no-network --no-cache --repositories-file /dev/null \
    --no-scripts add "$@" || fail 'APK closure preflight failed'
inventory "$work/root/lib/apk/db/installed" > "$work/preflight.tsv"
preflight=$(sha256sum "$work/preflight.tsv" | cut -d ' ' -f 1)
[ "$preflight" = "$(field after-sha256)" ] || fail 'APK closure preflight inventory mismatch'
inventory > "$work/base-installed.tsv"
apk --no-network --no-cache --repositories-file /dev/null add "$@"
inventory > "$work/installed.tsv"
after=$(sha256sum "$work/installed.tsv" | cut -d ' ' -f 1)
[ "$after" = "$(field after-sha256)" ] || fail 'APK final inventory mismatch'
receipt=/usr/share/nyxid/build-inputs/apk/$(sha256sum "$catalog" | cut -d ' ' -f 1)
[ ! -e "$receipt" ] || fail 'duplicate APK package record'
mkdir -p "$receipt"
cp "$catalog" "$receipt/archives.tsv"
cp "$work/base-installed.tsv" "$work/installed.tsv" "$receipt/"
