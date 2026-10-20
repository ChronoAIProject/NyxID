#!/bin/sh
# Every source must match the reviewed checksum. Missing durable assets may
# use provenance bytes; corrupted or unavailable durable assets fail closed.
set -eu
[ "$#" = 5 ] || exit 1
durable=$1 upstream=$2 destination=$3 mode=$4 sha=$5
case "$mode" in prefer-durable|provision-upstream) ;; *) exit 1 ;; esac
log=$(mktemp /tmp/nyxid-apk-fetch.XXXXXXXX)
trap 'rm -f -- "$log"' EXIT HUP INT TERM
# GitHub release downloads redirect to its dedicated release-asset CDN. Reject
# every other origin (including HTTP/credentials/lookalikes), even on a 404.
check_redirects() {
    awk 'tolower($1)=="location:" {
      url=$2
      if(url ~ /^https:\/\/github[.]com\// || url ~ /^https:\/\/release-assets[.]githubusercontent[.]com\/github-production-release-asset\//) next
      if(url ~ /^\/[^\/]/ && url !~ /\\/) next
      bad=1
    } END {exit bad}' "$log" || {
        printf '%s\n' 'APK durable redirect off-origin; upstream fallback forbidden' >&2
        exit 1
    }
}
selected=$durable
if [ "$mode" = provision-upstream ]; then
    printf '%s\n' "APK PROVISION: verifying upstream bytes for durable storage: $upstream" >&2
    selected=$upstream
    wget -S -T 120 -O "$destination" "$upstream" 2> "$log" || { cat "$log" >&2; exit 1; }
else
    status_code=0
    wget -S -T 120 -O "$destination" "$durable" 2> "$log" || status_code=$?
    check_redirects
    if [ "$status_code" != 0 ]; then
        status=$(awk '$1 ~ /^HTTP\/[0-9.]+$/ && $2 ~ /^[0-9][0-9][0-9]$/ {status=$2} END {print status}' "$log")
        if [ "$status" != 404 ]; then
            cat "$log" >&2
            printf '%s\n' 'APK durable download failed; upstream fallback is permitted only on HTTP 404' >&2
            exit 1
        fi
        tag=${durable#*/releases/download/}
        tag=${tag%%/*}
        printf '%s\n' "::warning::APK durable archives missing from $tag (HTTP 404); trying recorded upstream with mandatory checksum/signature/identity checks: $upstream" >&2
        selected=$upstream
        wget -S -T 120 -O "$destination" "$upstream" 2> "$log" || { cat "$log" >&2; exit 1; }
    fi
fi
printf '%s  %s\n' "$sha" "$destination" | sha256sum -c - >/dev/null || {
    printf '%s\n' 'APK archive checksum mismatch; no source fallback after integrity failure' >&2
    exit 1
}
printf '%s\n' "APK SOURCE: $selected" >&2
