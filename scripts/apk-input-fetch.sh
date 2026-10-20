#!/bin/sh
# Source selection never relaxes the catalog checksum. Normal builds use only
# repository-owned release assets; PR/bootstrap modes require explicit opt-in.
set -eu
[ "$#" = 5 ] || exit 1
durable=$1 upstream=$2 destination=$3 mode=$4 sha=$5
case "$mode" in durable|pr-upstream-if-missing|provision-upstream) ;; *) exit 1 ;; esac
log=$(mktemp /tmp/nyxid-apk-fetch.XXXXXXXX)
trap 'rm -f -- "$log"' EXIT HUP INT TERM
selected=$durable
if [ "$mode" = provision-upstream ]; then
    printf '%s\n' "APK PROVISION: verifying upstream bytes for durable storage: $upstream" >&2
    selected=$upstream
    wget -S -T 120 -O "$destination" "$upstream" 2> "$log" || { cat "$log" >&2; exit 1; }
elif ! wget -S -T 120 -O "$destination" "$durable" 2> "$log"; then
    status=$(awk '$1 ~ /^HTTP\/[0-9.]+$/ && $2 ~ /^[0-9][0-9][0-9]$/ {status=$2} END {print status}' "$log")
    if [ "$mode" != pr-upstream-if-missing ] || [ "$status" != 404 ]; then
        cat "$log" >&2
        printf '%s\n' 'APK durable download failed; upstream fallback is not permitted for this failure' >&2
        exit 1
    fi
    printf '%s\n' "APK PR FALLBACK (durable asset HTTP 404): $durable -> $upstream" >&2
    selected=$upstream
    wget -S -T 120 -O "$destination" "$upstream" 2> "$log" || { cat "$log" >&2; exit 1; }
fi
printf '%s  %s\n' "$sha" "$destination" | sha256sum -c - >/dev/null || {
    printf '%s\n' 'APK archive checksum mismatch; no source fallback after integrity failure' >&2
    exit 1
}
printf '%s\n' "APK SOURCE: $selected" >&2
