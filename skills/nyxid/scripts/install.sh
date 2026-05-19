#!/usr/bin/env bash
# SECURITY MANIFEST:
# Environment variables accessed: HOME, SHELL, PATH, CARGO_HOME, XDG_CONFIG_HOME,
#   XDG_DATA_HOME, NYXID_INSTALL_ROOT, NYXID_ACTIVE_SYMLINK
# External endpoints called: github.com (prebuilt installer), sh.rustup.rs
#   (fallback Rust installer), github.com (fallback cargo install)
# Local files read: shell RC files (~/.zshrc, ~/.bashrc, etc.)
# Local files written: shell RC files (adds ~/.local/bin if missing),
#   ~/.local/bin/nyxid, ~/.local/share/nyxid/versions/<version>/nyxid
#
# NyxID CLI installer -- prefers the prebuilt cargo-dist binary installer and
# only falls back to cargo install when the host platform has no release asset.
set -euo pipefail

REPO="https://github.com/ChronoAIProject/NyxID"
INSTALLER_URL="https://github.com/ChronoAIProject/NyxID/releases/latest/download/nyxid-cli-installer.sh"
LOCAL_BIN="$HOME/.local/bin"
ACTIVE_NYXID="${NYXID_ACTIVE_SYMLINK:-$LOCAL_BIN/nyxid}"
VERSIONS_ROOT="${NYXID_INSTALL_ROOT:-${XDG_DATA_HOME:-$HOME/.local/share}/nyxid/versions}"
CARGO_HOME_DIR="${CARGO_HOME:-$HOME/.cargo}"
CARGO_BIN="$CARGO_HOME_DIR/bin"
CARGO_ENV="$CARGO_HOME_DIR/env"

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

info() { printf '  %s\n' "$*" >&2; }
warn() { printf '  [warn] %s\n' "$*" >&2; }
fail() {
  printf '  [error] %s\n' "$*" >&2
  exit 1
}

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
  [ -f "$rc_file" ] || return 1

  grep -Fq "$LOCAL_BIN" "$rc_file" 2>/dev/null && return 0
  grep -Eq '(\$HOME|\$\{HOME\}|~)/\.local/bin|fish_add_path.*\.local/bin' "$rc_file" 2>/dev/null
}

ensure_local_bin_path() {
  local rc_file shell_name
  rc_file="$(detect_shell_rc)"
  shell_name="$(basename "${SHELL:-/bin/sh}")"

  if path_in_rc "$rc_file"; then
    info "PATH already configured in $rc_file"
    return
  fi

  info "Adding $LOCAL_BIN to PATH in $rc_file..."
  mkdir -p "$(dirname "$rc_file")"
  {
    echo ""
    echo "# NyxID CLI"
    if [ "$shell_name" = "fish" ]; then
      printf 'fish_add_path "%s"\n' "$LOCAL_BIN"
    else
      printf 'export PATH="%s:$PATH"\n' "$LOCAL_BIN"
    fi
  } >> "$rc_file"

  info "Done -- $rc_file updated."
  info "Open a new terminal or run: source $rc_file"
}

prebuilt_target_supported() {
  local os arch
  os="$(uname -s)"
  arch="$(uname -m)"

  case "$os:$arch" in
    Linux:x86_64 | Linux:amd64 | Linux:aarch64 | Linux:arm64)
      return 0
      ;;
    Darwin:x86_64 | Darwin:arm64 | Darwin:aarch64)
      return 0
      ;;
    *)
      return 1
      ;;
  esac
}

install_prebuilt() {
  mkdir -p "$LOCAL_BIN"
  info "Installing NyxID CLI prebuilt binary..."

  if curl --proto '=https' --tlsv1.2 -fsSL "$INSTALLER_URL" | sh; then
    if [ -x "$ACTIVE_NYXID" ]; then
      migrate_prebuilt_to_versioned_layout
      info "NyxID CLI installed at $ACTIVE_NYXID"
      return 0
    fi

    warn "prebuilt installer completed but $ACTIVE_NYXID was not found"
  else
    warn "prebuilt installer failed"
  fi

  return 1
}

detect_nyxid_version() {
  "$ACTIVE_NYXID" --version 2>/dev/null \
    | grep -Eo 'v?[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.-]+)?' \
    | head -n 1
}

migrate_prebuilt_to_versioned_layout() {
  local raw_version version version_dir versioned_bin active_dir tmp_link
  raw_version="$(detect_nyxid_version || true)"
  if [ -z "$raw_version" ]; then
    fail "could not determine nyxid version from $ACTIVE_NYXID --version"
  fi

  case "$raw_version" in
    v*) version="$raw_version" ;;
    *) version="v$raw_version" ;;
  esac

  version_dir="$VERSIONS_ROOT/$version"
  versioned_bin="$version_dir/nyxid"
  active_dir="$(dirname "$ACTIVE_NYXID")"
  tmp_link="$active_dir/nyxid.tmp.$$"

  mkdir -p "$version_dir" "$active_dir"
  install -m 755 "$ACTIVE_NYXID" "$versioned_bin"

  rm -f "$tmp_link"
  ln -s "$versioned_bin" "$tmp_link"
  mv -f "$tmp_link" "$ACTIVE_NYXID"

  info "Versioned install: $versioned_bin"
}

install_from_source() {
  info "Falling back to source install. This requires Rust and may take several minutes."

  if command -v cargo &>/dev/null; then
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

  if ! command -v cargo &>/dev/null; then
    fail "cargo still not found after setup. Please add $CARGO_BIN to your PATH manually."
  fi

  cargo install --git "$REPO" nyxid-cli --force --locked

  if [ ! -x "$CARGO_BIN/nyxid" ]; then
    fail "cargo install completed but $CARGO_BIN/nyxid was not found"
  fi

  mkdir -p "$LOCAL_BIN"
  install -m 755 "$CARGO_BIN/nyxid" "$LOCAL_BIN/nyxid"
  info "NyxID CLI installed at $LOCAL_BIN/nyxid"
}

# ---------------------------------------------------------------------------
# Install
# ---------------------------------------------------------------------------

if prebuilt_target_supported; then
  if ! install_prebuilt; then
    warn "No usable prebuilt binary was available for this host; using source fallback."
    install_from_source
  fi
else
  warn "No prebuilt NyxID CLI binary is published for $(uname -s)/$(uname -m)."
  install_from_source
fi

ensure_local_bin_path

# ---------------------------------------------------------------------------
# Verify
# ---------------------------------------------------------------------------

if [ -x "$LOCAL_BIN/nyxid" ]; then
  info "Verified: $("$LOCAL_BIN/nyxid" --version 2>/dev/null || echo 'nyxid is available')"
else
  fail "nyxid binary not found -- installation may have failed"
fi

info ""
info "Installation complete!"
