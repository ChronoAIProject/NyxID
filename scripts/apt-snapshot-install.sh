#!/bin/sh
# Immutable APT closure: pinned signed InRelease -> authenticated indexes ->
# authenticated downloaded .debs -> offline install. Requires only base APT,
# dpkg, awk and coreutils, including in the slim runtime images.
set -eu

fail() { printf '%s\n' "$*" >&2; exit 1; }
upgrade_base=true
# Hosted development runners are not shipped image roots. Install their exact
# requested dependencies from the same authenticated snapshots without a host
# OS upgrade. Dockerfile admission requires the default image-upgrade form.
if [ "${1:-}" = --install-only ]; then upgrade_base=false; shift; fi
[ "$#" -ge 4 ] || fail 'usage: apt-snapshot-install.sh [--install-only] CATALOG DISTRO:CODENAME [...] -- PACKAGE [...]'
catalog=$1
shift
[ -f "$catalog" ] || fail 'snapshot catalog is missing'
[ "$(id -u)" = 0 ] || fail 'APT snapshot installation requires image-build root'

work=$(mktemp -d /tmp/nyxid-apt.XXXXXXXX)
trap 'rm -rf -- "$work"' EXIT HUP INT TERM
# Public package inputs need to be accessible to APT's existing _apt sandbox.
chmod 0755 "$work"
selected=$work/selected.tsv
selectors=
while [ "$#" -gt 0 ] && [ "$1" != -- ]; do
    case "$1" in
        debian:bookworm|debian:trixie|ubuntu:focal|ubuntu:noble) ;;
        *) fail 'unsupported snapshot selector' ;;
    esac
    selectors="${selectors:+$selectors }$1"
    shift
done
[ "$#" -gt 1 ] && [ "$1" = -- ] || fail 'explicit package list is required'
shift
for package in "$@"; do
    case "$package" in
        ''|-*|*[!a-z0-9+.:=-]*) fail 'invalid explicit package name' ;;
    esac
done
# Reject incomplete requested roots, duplicate selectors and malformed catalog
# entries before APT can touch indexes, download archives or mutate packages.
snapshot_script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
awk -v selectors="$selectors" -f "$snapshot_script_dir/apt-snapshot-select.awk" "$catalog" > "$selected" \
    || fail 'invalid or incomplete snapshot metadata selection'

# This public Mozilla bundle is transport-only, never installed into the image's
# trust store. Ubuntu redirects even HTTP snapshots to HTTPS on minimal bases.
# Verify reviewed bytes before any APT request; distro signatures plus the exact
# InRelease hash remain mandatory independently of TLS.
transport_ca=$(CDPATH= cd -- "$(dirname -- "$catalog")" && pwd)/apt-transport-ca.pem
[ -f "$transport_ca" ] && [ ! -L "$transport_ca" ] || fail 'APT transport CA bundle missing or symlinked'
transport_pins=${transport_ca%.pem}.tsv
[ -f "$transport_pins" ] && [ ! -L "$transport_pins" ] || fail 'APT transport CA provenance missing or symlinked'
transport_digest=$(awk -f "$snapshot_script_dir/apt-transport-ca.awk" "$transport_pins") \
    || fail 'invalid APT transport CA provenance'
printf '%s  %s\n' "$transport_digest" "$transport_ca" | sha256sum -c - >/dev/null \
    || fail 'APT transport CA bundle checksum mismatch'
# Hosted checkout parents may be private to the runner. APT's _apt acquisition
# user must traverse the public temporary directory, never the source checkout.
cp "$transport_ca" "$work/transport-ca.pem"
chmod 0644 "$work/transport-ca.pem"
transport_ca=$work/transport-ca.pem
printf '%s  %s\n' "$transport_digest" "$transport_ca" | sha256sum -c - >/dev/null \
    || fail 'APT transport CA copy checksum mismatch'

mkdir "$work/lists" "$work/lists/partial" "$work/archives" "$work/archives/partial"
sources=$work/sources.list
: > "$sources"
tab=$(printf '\t')
while IFS="$tab" read -r distro codename suite components url digest; do
    key=/usr/share/keyrings/$distro-archive-keyring.gpg
    [ -s "$key" ] || fail 'authenticated base archive keyring is required'
    # Fixed snapshots deliberately outlive Release.Valid-Until. Disable that
    # freshness field only for these SHA-pinned signed roots. Signature checks
    # and APT index/package checksums remain mandatory; scans assess currency.
    # Every caller now supplies transport trust. Use HTTPS directly: Ubuntu's
    # HTTP->HTTPS redirect otherwise repeats for hundreds of small archives.
    printf 'deb [signed-by=%s check-valid-until=no] %s %s %s\n' \
        "$key" "$url" "$suite" "$(printf '%s' "$components" | tr ',' ' ')" >> "$sources"
done < "$selected"

apt_snapshot() {
    apt-get -o "Dir::Etc::sourcelist=$sources" -o 'Dir::Etc::sourceparts=-' \
        -o "Dir::State::lists=$work/lists" -o "Dir::Cache::archives=$work/archives" \
        -o APT::Update::Error-Mode=any -o APT::Get::AllowUnauthenticated=false \
        -o Acquire::AllowInsecureRepositories=false -o Acquire::AllowDowngradeToInsecureRepositories=false \
        -o "Acquire::https::CaInfo=$transport_ca" \
        -o Acquire::https::Verify-Peer=true -o Acquire::https::Verify-Host=true \
        -o Acquire::Languages=none -o Acquire::Retries=2 \
        -o Acquire::http::Timeout=30 -o Acquire::https::Timeout=30 "$@"
}
architecture=$(dpkg --print-architecture)
case "$architecture" in amd64|arm64) ;; *) fail 'APT architecture has no reviewed build coverage' ;; esac
[ -z "$(dpkg --print-foreign-architectures)" ] || fail 'foreign APT architectures require an explicit reviewed closure'
apt_snapshot update
while IFS="$tab" read -r distro codename suite components url digest; do
    # APT encodes these catalog hosts/paths as this exact cache basename.
    cache=$(printf '%s/dists/%s/InRelease' "${url#https://}" "$suite" | tr '/' '_')
    [ -f "$work/lists/$cache" ] || fail 'required signed snapshot metadata was not fetched'
    if ! printf '%s  %s\n' "$digest" "$work/lists/$cache" | sha256sum -c - >/dev/null; then
        actual=$(sha256sum "$work/lists/$cache")
        actual=${actual%% *}
        fail "signed snapshot bytes differ from the reviewed catalog: $distro:$suite $url/dists/$suite/InRelease expected=$digest actual=$actual"
    fi
    # Old APT versions can report a failed optional index as a warning. Require
    # every selected component's native-architecture Packages index explicitly;
    # an empty authenticated index still exists as a compressed cache file.
    prefix=${cache%InRelease}
    for component in $(printf '%s' "$components" | tr ',' ' '); do
        count=0
        for index in "$work/lists/${prefix}${component}_binary-${architecture}_Packages"*; do
            [ -f "$index" ] || continue
            case "${index##*Packages}" in ''|.lz4|.gz|.xz|.bz2) ;;
                *) fail 'unexpected package-index cache encoding' ;;
            esac
            count=$((count + 1))
        done
        [ "$count" = 1 ] || fail 'required authenticated package index is missing or ambiguous'
    done
done < "$selected"

dpkg-query -W -f='${Package}\t${Version}\t${Architecture}\t${db:Status-Status}\n' \
    | LC_ALL=C sort > "$work/before.tsv"
# Inherited images may invoke the same snapshot roots again with different
# additions. Preserve each invocation, including its base package state, rather
# than overwriting the desktop's earlier Chromium/package evidence.
root_digest=$({ cat "$selected"; printf '%s\n' '# requested packages' "$@" '# base packages'; cat "$work/before.tsv"; } \
    | sha256sum | cut -d ' ' -f 1)
evidence=/usr/share/nyxid/build-inputs/apt/$root_digest
mkdir -p "$evidence"
cp "$selected" "$evidence/snapshots.tsv"
cp "$work/before.tsv" "$evidence/base-installed-packages.tsv"
printf '%s\n' "$@" > "$evidence/requested-packages.txt"
printf '%s\n' 'nyxid.apt-snapshot-install.v1' > "$evidence/schema"
dpkg --print-architecture > "$evidence/architecture"
: > "$evidence/fetched-debs.tsv"
record_debs() {
    for archive in "$work/archives/"*.deb; do
        [ -f "$archive" ] || continue
        sha=$(sha256sum "$archive" | cut -d ' ' -f 1)
        printf '%s\t%s\t%s\t%s\n' "$(dpkg-deb -f "$archive" Package)" \
            "$(dpkg-deb -f "$archive" Version)" "$(dpkg-deb -f "$archive" Architecture)" "$sha" \
            >> "$evidence/fetched-debs.tsv"
    done
}
# Upgrade inherited packages as well as installing explicit additions. Each
# download is verified against the signed, pinned indexes. --no-download makes
# both install operations fail closed if any authenticated package is missing.
export DEBIAN_FRONTEND=noninteractive
if [ "$upgrade_base" = true ]; then
    apt_snapshot -y --no-install-recommends --download-only upgrade
    record_debs
    apt_snapshot -y --no-install-recommends --no-download upgrade
fi
apt_snapshot -y --no-install-recommends --download-only install "$@"
record_debs
apt_snapshot -y --no-install-recommends --no-download install "$@"
LC_ALL=C sort -u "$evidence/fetched-debs.tsv" -o "$evidence/fetched-debs.tsv"
dpkg-query -W -f='${Package}\t${Version}\t${Architecture}\t${db:Status-Status}\n' \
    | LC_ALL=C sort > "$evidence/installed-packages.tsv"
mkdir -p "$evidence/metadata"
for release in "$work/lists/"*InRelease; do
    cp "$release" "$evidence/metadata/"
done
sha256sum "$evidence"/metadata/* > "$evidence/metadata-sha256.txt"
