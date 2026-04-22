#!/usr/bin/env bash
# SECURITY MANIFEST:
# Environment variables accessed: HOME, SHELL, PATH, CARGO_HOME
# External endpoints called: github.com
# Local files read: shell RC files (~/.zshrc, ~/.bashrc, etc.)
# Local files written: shell RC files, ~/.local/bin/nyxid, ~/.cargo/bin/nyxid (optional symlink)
#
# NyxID CLI installer -- downloads a prebuilt release by default, with an
# opt-in source-build fallback for unsupported platforms or power users.
set -euo pipefail

REPO_SLUG="ChronoAIProject/NyxID"
RELEASES_BASE="https://github.com/$REPO_SLUG/releases"
REPO_URL="https://github.com/$REPO_SLUG.git"
COSIGN_ISSUER="https://token.actions.githubusercontent.com"

LOCAL_BIN="$HOME/.local/bin"
CARGO_HOME_DIR="${CARGO_HOME:-$HOME/.cargo}"
CARGO_BIN="$CARGO_HOME_DIR/bin"
CARGO_ENV="$CARGO_HOME_DIR/env"

FROM_SOURCE=0

info()  { printf '  %s\n' "$*" >&2; }
warn()  { printf '  [warn] %s\n' "$*" >&2; }
fail()  { printf '  [error] %s\n' "$*" >&2; exit 1; }

usage() {
  cat >&2 <<'EOF'
Usage: install.sh [--from-source]

Options:
  --from-source   Build via cargo install instead of downloading a prebuilt binary
EOF
  exit 1
}

for arg in "$@"; do
  case "$arg" in
    --from-source)
      FROM_SOURCE=1
      ;;
    -h|--help)
      usage
      ;;
    *)
      fail "Unknown argument: $arg"
      ;;
  esac
done

detect_shell_rc() {
  local shell_name
  shell_name="$(basename "${SHELL:-/bin/sh}")"

  case "$shell_name" in
    zsh)
      echo "$HOME/.zshrc"
      ;;
    bash)
      if [ "$(uname)" = "Darwin" ]; then
        echo "$HOME/.bash_profile"
      else
        echo "$HOME/.bashrc"
      fi
      ;;
    fish)
      echo "${XDG_CONFIG_HOME:-$HOME/.config}/fish/config.fish"
      ;;
    *)
      echo "$HOME/.profile"
      ;;
  esac
}

path_in_rc() {
  local rc_file="$1"
  local path_value="$2"
  [ -f "$rc_file" ] || return 1

  grep -Fq "$path_value" "$rc_file" 2>/dev/null
}

cargo_in_rc() {
  local rc_file="$1"
  [ -f "$rc_file" ] || return 1

  grep -Fq "$CARGO_BIN" "$rc_file" 2>/dev/null && return 0
  grep -Fq "$CARGO_ENV" "$rc_file" 2>/dev/null && return 0
  grep -Eq '(\$HOME|\$\{HOME\}|~)/\.cargo/(bin|env)|\.cargo/(bin|env)|fish_add_path' "$rc_file" 2>/dev/null
}

detect_target() {
  local platform
  platform="$(uname -sm)"

  case "$platform" in
    "Linux x86_64")
      echo "x86_64-unknown-linux-gnu"
      ;;
    "Linux aarch64"|"Linux arm64")
      echo "aarch64-unknown-linux-gnu"
      ;;
    "Darwin x86_64")
      echo "x86_64-apple-darwin"
      ;;
    "Darwin arm64")
      echo "aarch64-apple-darwin"
      ;;
    *)
      return 1
      ;;
  esac
}

resolve_latest_tag() {
  curl -fsSLI -o /dev/null -w '%{url_effective}' "$RELEASES_BASE/latest" | awk -F/ '{print $NF}'
}

download_file() {
  local url="$1"
  local dest="$2"
  curl -fsSL --retry 3 --retry-delay 1 "$url" -o "$dest"
}

verify_checksum() {
  local checksums_file="$1"
  local archive_file="$2"
  local archive_name checksum_entry

  archive_name="$(basename "$archive_file")"
  checksum_entry="$(awk -v file="$archive_name" '$2 == file || $2 == ("*" file) { print; exit }' "$checksums_file")"
  [ -n "$checksum_entry" ] || fail "SHA256SUMS does not contain an entry for $archive_name."

  if command -v sha256sum >/dev/null 2>&1; then
    (
      cd "$(dirname "$archive_file")"
      printf '%s\n' "$checksum_entry" | sha256sum -c -
    )
    return
  fi

  if command -v shasum >/dev/null 2>&1; then
    (
      cd "$(dirname "$archive_file")"
      printf '%s\n' "$checksum_entry" | shasum -a 256 -c -
    )
    return
  fi

  fail "sha256sum or shasum is required to verify the release checksum."
}

verify_cosign_signature() {
  local tag="$1"
  local checksums_file="$2"
  local signature_file="$3"
  local cert_file="$4"
  local identity="https://github.com/$REPO_SLUG/.github/workflows/release.yml@refs/tags/$tag"

  if ! command -v cosign >/dev/null 2>&1; then
    warn "cosign not found -- skipping signature verification for SHA256SUMS"
    return
  fi

  info "Verifying SHA256SUMS signature with cosign..."
  if ! cosign verify-blob \
    --certificate-identity "$identity" \
    --certificate-oidc-issuer "$COSIGN_ISSUER" \
    --certificate "$cert_file" \
    --signature "$signature_file" \
    "$checksums_file" >/dev/null; then
    fail "cosign verification failed for SHA256SUMS"
  fi
}

find_extracted_binary() {
  local root="$1"
  find "$root" -type f -name nyxid -print | head -n1
}

install_prebuilt() {
  local target tag version archive_name archive_url tmp_dir checksums_file signature_file cert_file extract_dir extracted_binary

  target="$(detect_target)" || fail "No prebuilt NyxID binary is published for $(uname -sm). Re-run with --from-source."
  tag="$(resolve_latest_tag)"
  version="${tag#v}"
  archive_name="nyxid-${version}-${target}.tar.xz"
  archive_url="$RELEASES_BASE/download/$tag/$archive_name"

  tmp_dir="$(mktemp -d "${TMPDIR:-/tmp}/nyxid-install.XXXXXX")"
  trap 'rm -rf "$tmp_dir"' EXIT

  checksums_file="$tmp_dir/SHA256SUMS"
  signature_file="$tmp_dir/SHA256SUMS.sig"
  cert_file="$tmp_dir/SHA256SUMS.pem"

  info "Downloading NyxID CLI $version for $target..."
  download_file "$archive_url" "$tmp_dir/$archive_name"
  download_file "$RELEASES_BASE/download/$tag/SHA256SUMS" "$checksums_file"
  download_file "$RELEASES_BASE/download/$tag/SHA256SUMS.sig" "$signature_file"
  download_file "$RELEASES_BASE/download/$tag/SHA256SUMS.pem" "$cert_file"

  verify_cosign_signature "$tag" "$checksums_file" "$signature_file" "$cert_file"

  info "Verifying archive checksum..."
  verify_checksum "$checksums_file" "$tmp_dir/$archive_name"

  extract_dir="$tmp_dir/extract"
  mkdir -p "$extract_dir"
  tar -xJf "$tmp_dir/$archive_name" -C "$extract_dir"

  extracted_binary="$(find_extracted_binary "$extract_dir")"
  [ -n "$extracted_binary" ] || fail "Downloaded archive did not contain a nyxid binary."

  mkdir -p "$LOCAL_BIN"
  install -m 0755 "$extracted_binary" "$LOCAL_BIN/nyxid"
  info "Installed NyxID CLI at $LOCAL_BIN/nyxid"

  if [ -d "$CARGO_BIN" ]; then
    ln -sf "$LOCAL_BIN/nyxid" "$CARGO_BIN/nyxid"
    info "Symlinked $CARGO_BIN/nyxid -> $LOCAL_BIN/nyxid"
  fi
}

install_from_source() {
  if command -v cargo >/dev/null 2>&1; then
    info "Rust toolchain already installed ($(cargo --version))"
  else
    info "Rust toolchain not found -- installing via rustup..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    info "Rust installed successfully."
  fi

  if [ -f "$CARGO_ENV" ]; then
    # shellcheck disable=SC1090
    . "$CARGO_ENV"
  else
    export PATH="$CARGO_BIN:$PATH"
  fi

  command -v cargo >/dev/null 2>&1 || fail "cargo still not found after setup. Please add $CARGO_BIN to PATH manually."

  info "Installing NyxID CLI from source..."
  cargo install --git "$REPO_URL" nyxid-cli --force --locked

  mkdir -p "$LOCAL_BIN"
  if [ -x "$CARGO_BIN/nyxid" ]; then
    ln -sf "$CARGO_BIN/nyxid" "$LOCAL_BIN/nyxid"
    info "Symlinked $LOCAL_BIN/nyxid -> $CARGO_BIN/nyxid"
  else
    fail "nyxid binary not found in $CARGO_BIN after cargo install"
  fi
}

configure_path() {
  local rc_file shell_name
  rc_file="$(detect_shell_rc)"
  shell_name="$(basename "${SHELL:-/bin/sh}")"

  mkdir -p "$(dirname "$rc_file")"

  if path_in_rc "$rc_file" "$LOCAL_BIN"; then
    info "PATH already contains $LOCAL_BIN in $rc_file"
  else
    info "Adding $LOCAL_BIN to PATH in $rc_file..."
    {
      echo ""
      echo "# NyxID CLI -- added by installer"
      if [ "$shell_name" = "fish" ]; then
        printf 'fish_add_path "%s"\n' "$LOCAL_BIN"
      else
        printf 'export PATH="%s:$PATH"\n' "$LOCAL_BIN"
      fi
    } >> "$rc_file"
    info "Done -- $rc_file updated."
  fi

  if [ "$FROM_SOURCE" -eq 1 ]; then
    if cargo_in_rc "$rc_file"; then
      info "Cargo PATH already configured in $rc_file"
    else
      info "Adding cargo to PATH in $rc_file..."
      {
        echo ""
        echo "# Cargo (Rust package manager) -- added by NyxID installer"
        if [ "$shell_name" = "fish" ]; then
          printf 'fish_add_path "%s"\n' "$CARGO_BIN"
        elif [ -f "$CARGO_ENV" ]; then
          printf '. "%s"\n' "$CARGO_ENV"
        else
          printf 'export PATH="%s:$PATH"\n' "$CARGO_BIN"
        fi
      } >> "$rc_file"
      info "Done -- $rc_file updated."
    fi
  fi
}

verify_install() {
  if command -v nyxid >/dev/null 2>&1; then
    info "Verified: $(nyxid --version 2>/dev/null || echo 'nyxid is available')"
  elif [ -x "$LOCAL_BIN/nyxid" ]; then
    info "Installed at $LOCAL_BIN/nyxid (open a new terminal if it's not yet in PATH)"
  else
    fail "nyxid binary not found after installation"
  fi
}

if [ "$FROM_SOURCE" -eq 1 ]; then
  install_from_source
else
  install_prebuilt
fi

configure_path
verify_install

info ""
info "Installation complete!"
