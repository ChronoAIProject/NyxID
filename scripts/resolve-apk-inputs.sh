#!/bin/sh
# Deliberate input refresh only. Run inside the reviewed digest-pinned base,
# with /out mounted for candidate receipts. Product builds use the resulting
# reviewed archive catalog through apk-input-install.sh, never this resolver.
set -eu
export LC_ALL=C
[ "$(id -u)" = 0 ] && [ "$#" -ge 2 ] || exit 1
base=$1
shift
case "$base" in *@sha256:*) ;; *) exit 1 ;; esac
for package in "$@"; do
    case "$package" in ''|-*|*[!a-z0-9+.-]*) exit 1 ;; esac
done
mkdir -p /out/archives /out/indexes
# Optional isolated cross-architecture solver root, for candidate generation
# only. No target binaries/scripts execute. Native PR replay is authoritative.
resolve_root=${NYXID_APK_RESOLVE_ROOT:-}
resolve_arch=${NYXID_APK_RESOLVE_ARCH:-$(apk --print-arch)}
apk() {
    if [ -n "$resolve_root" ]; then
        operation=$1
        shift
        case "$operation" in
            add) command apk --root "$resolve_root" --arch "$resolve_arch" add --no-scripts "$@" ;;
            upgrade)
                case "$(command apk --version)" in
                    'apk-tools 3.'*) self_flag=--no-preupgrade ;;
                    *) self_flag=--no-self-upgrade ;;
                esac
                command apk --root "$resolve_root" --arch "$resolve_arch" upgrade --no-scripts "$self_flag" "$@" ;;
            *) command apk --root "$resolve_root" --arch "$resolve_arch" "$operation" "$@" ;;
        esac
    else
        command apk "$@"
    fi
}
inventory() {
    awk -F: '
        /^P:/ {p=$2} /^V:/ {v=$2} /^A:/ {a=$2}
        /^$/ {if(p!="") print p "\t" v "\t" a; p=""}
        END {if(p!="") print p "\t" v "\t" a}
    ' "$resolve_root/lib/apk/db/installed" | LC_ALL=C sort
}
inventory > /out/base-installed.tsv
cp "$resolve_root/etc/apk/repositories" /out/repositories.txt
apk update
cp "$resolve_root"/var/cache/apk/APKINDEX*.tar.gz /out/indexes/
sha256sum /out/indexes/* > /out/indexes-sha256.txt
apk upgrade
apk add "$@"
inventory > /out/resolved-installed.tsv
native_arch=$resolve_arch
awk -F '\t' 'NR==FNR {old[$1]=$0;next} old[$1]!=$0 {print}' \
    /out/base-installed.tsv /out/resolved-installed.tsv > /out/changed.tsv
awk -F '\t' 'NR==FNR {present[$1]=1;next} !present[$1] {exit 1}' \
    /out/resolved-installed.tsv /out/base-installed.tsv
catalog=/out/archives.tsv
rows=/out/archive-inputs.tsv
: > "$rows"
refreshed=${NYXID_APK_REFRESH_DATE:-$(date -u +%Y-%m-%d)}
tab=$(printf '\t')
while IFS="$tab" read -r name version architecture; do
    # Pinned APK 2/3 expose query-mode fetch. Exclude the installed DB so
    # already-installed packages and meta-packages still produce a repo URL.
    # --simulate prints URLs without downloading to the working directory.
    case "$(apk --version)" in
        'apk-tools 3.'*|'apk-tools 2.'*)
            urls=$(apk fetch --simulate --from repositories --url "$name") ;;
        *) printf '%s\n' 'unsupported APK resolver version' >&2; exit 1 ;;
    esac
    url=$(printf '%s\n' "$urls" | awk -F / -v archive="$name-$version.apk" '$NF==archive {print;count++} END {if(count!=1) exit 1}')
    case "$architecture" in "$native_arch"|noarch) ;; *) exit 1 ;; esac
    case "$url" in https://dl-cdn.alpinelinux.org/alpine/*/"$native_arch"/"$name-$version.apk") ;;
        *) printf '%s\n' 'unexpected APK archive origin' >&2; exit 1 ;;
    esac
    archive=/out/archives/$name-$version.apk
    wget -T 120 -O "$archive" "$url"
    apk verify "$archive"
    # APK 3's repository installation stamps native arch even on some signed
    # noarch meta packages. Explicit local-archive replay retains their true
    # arch. Keep the raw resolver DB separately and bind the expected replay
    # inventory to the verified archive's own name/version/architecture.
    metadata=$(tar -xzOf "$archive" .PKGINFO)
    archive_arch=$(printf '%s\n' "$metadata" | awk -F ' = ' -v name="$name" -v version="$version" '
        $1=="pkgname" {if($2!=name) exit 1; names++}
        $1=="pkgver" {if($2!=version) exit 1; versions++}
        $1=="arch" {print $2; architectures++}
        END {if(names!=1 || versions!=1 || architectures!=1) exit 1}')
    case "$archive_arch" in "$native_arch"|noarch) ;; *) exit 1 ;; esac
    sha=$(sha256sum "$archive" | cut -d ' ' -f 1)
    durable=https://github.com/ChronoAIProject/NyxID/releases/download/build-inputs-apk-$refreshed/$sha-$archive_arch-$name-$version.apk
    printf '%s\t%s\t%s\t%s\t%s\t%s\n' "$name" "$version" "$archive_arch" "$sha" "$durable" "$url" >> "$rows"
done < /out/changed.tsv
awk -F '\t' 'BEGIN {OFS="\t"} NR==FNR {arch[$1]=$3;next} {if($1 in arch) $3=arch[$1];print}' \
    "$rows" /out/resolved-installed.tsv > /out/installed.tsv
printf '# nyxid.apk-inputs.v2\n# base-image %s\n# arch %s\n# refreshed-on %s\n# packages %s\n# before-sha256 %s\n# after-sha256 %s\n' \
    "$base" "$native_arch" "$refreshed" "$(printf '%s\n' "$@" | LC_ALL=C sort -u | tr '\n' ' ' | sed 's/ $//')" \
    "$(sha256sum /out/base-installed.tsv | cut -d ' ' -f 1)" \
    "$(sha256sum /out/installed.tsv | cut -d ' ' -f 1)" > "$catalog"
cat "$rows" >> "$catalog"
printf '%s\n' 'Candidate archive selection only; review catalog and signatures before build use.'
