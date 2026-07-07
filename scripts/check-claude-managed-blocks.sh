#!/usr/bin/env bash
set -euo pipefail

file="${1:-CLAUDE.md}"
block_name="foundational-invariants"
start_marker="consensus-rnd:${block_name}:start"
end_marker="consensus-rnd:${block_name}:end"

sha256_stdin() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum | awk '{ print $1 }'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 | awk '{ print $1 }'
    else
        echo "missing sha256sum or shasum" >&2
        exit 1
    fi
}

if [[ ! -f "$file" ]]; then
    echo "missing file: $file" >&2
    exit 1
fi

start_count=$(grep -c "<!-- ${start_marker} " "$file" || true)
end_count=$(grep -c "<!-- ${end_marker} -->" "$file" || true)

if [[ "$start_count" -ne 1 || "$end_count" -ne 1 ]]; then
    echo "expected exactly one ${block_name} start and end marker" >&2
    exit 1
fi

declared=$(
    sed -n "s/^<!-- ${start_marker} .*sha256=\([0-9a-f]\{64\}\).* -->$/\1/p" "$file"
)

if [[ -z "$declared" ]]; then
    echo "missing sha256 on ${block_name} start marker" >&2
    exit 1
fi

actual=$(
    awk -v start="$start_marker" -v end="$end_marker" '
        index($0, start) { in_block = 1; next }
        index($0, end) { in_block = 0 }
        in_block { print }
    ' "$file" | sha256_stdin
)

if [[ "$actual" != "$declared" ]]; then
    echo "${block_name} sha256 mismatch: declared ${declared}, actual ${actual}" >&2
    exit 1
fi

echo "${block_name} sha256 verified: ${actual}"
