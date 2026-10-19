#!/bin/sh
# Download authenticated reviewed archives, then replay with RUN --network=none.
set -eu
export LC_ALL=C
fail() { printf '%s\n' "$*" >&2; exit 1; }
[ "$#" = 4 ] && [ "$(id -u)" = 0 ] || fail 'usage: apk-input-install.sh --download|--install CATALOG BASE-IMAGE ARCHIVE-DIRECTORY (root)'
mode=$1 catalog=$2 base=$3 work=$4
case "$work" in /*/archives) ;; *) fail 'APK archive path must be an absolute archives directory' ;; esac
[ ! -L "$work" ] || fail 'APK archive directory must not be symlinked'
case "$mode" in --download|--install) ;; *) fail 'invalid APK installer mode' ;; esac
[ -f "$catalog" ] || fail 'APK catalog missing'
script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
architecture=$(apk --print-arch)
# Validate the entire catalog before creating output or making network requests.
rows=$(awk -v base="$base" -v architecture="$architecture" -f "$script_dir/apk-catalog.awk" "$catalog") || fail 'invalid APK catalog'
field() { awk -v key="$1" '$1=="#" && $2==key {print $3}' "$catalog"; }
inventory() { awk -f "$script_dir/apk-inventory.awk" /lib/apk/db/installed | LC_ALL=C sort; }
before=$(inventory | sha256sum | cut -d ' ' -f 1)
[ "$before" = "$(field before-sha256)" ] || fail 'APK base inventory mismatch'
if [ "$mode" = --download ]; then
    [ ! -e "$work" ] || fail 'APK archive directory must be fresh'
    mkdir -p "$work"
else
    [ -d "$work" ] || fail 'APK archives missing'
fi
set --
tab=$(printf '\t')
while IFS="$tab" read -r name version arch sha url; do
    archive=$work/$name-$version.apk
    if [ "$mode" = --download ]; then
        wget -T 120 -O "$archive" "$url"
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
# Ignore inherited indexes/cache/repositories: the complete closure consists
# only of the pinned base plus these explicit, signature-checked archives.
apk --no-network --no-cache --repositories-file /dev/null add --simulate "$@" || fail 'APK closure preflight failed'
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
rm -rf -- "$work"
