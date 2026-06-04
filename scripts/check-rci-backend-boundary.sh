#!/usr/bin/env bash
set -euo pipefail

forbidden_source='nyxid[-_]crypto|x25519[-_]dalek|chacha20poly1305|XChaCha20Poly1305|hkdf::|\bHkdf\b|\bhkdf\s*='
if rg -n "$forbidden_source" backend/Cargo.toml backend/src; then
  echo "backend must not contain RCI crypto crate/import/direct dependency references" >&2
  exit 1
fi

tree="$(cargo tree -p nyxid --no-default-features --edges normal,build)"
if grep -E 'nyxid-crypto|x25519-dalek|chacha20poly1305' <<<"$tree"; then
  echo "backend dependency graph contains RCI crypto packages" >&2
  exit 1
fi

all_feature_tree="$(cargo tree -p nyxid --all-features --edges normal,build)"
if grep -E 'nyxid-crypto' <<<"$all_feature_tree"; then
  echo "backend all-features graph contains nyxid-crypto" >&2
  exit 1
fi
