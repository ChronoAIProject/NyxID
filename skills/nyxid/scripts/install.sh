#!/usr/bin/env bash
# SECURITY MANIFEST:
# Environment variables accessed: HOME, SHELL, PATH, CARGO_HOME, XDG_CONFIG_HOME,
#   XDG_DATA_HOME, NYXID_INSTALL_ROOT, NYXID_ACTIVE_SYMLINK, CC, TMPDIR, PWD
# Subprocess controls: NYXID_CLI_UNMANAGED_INSTALL, NYXID_CLI_INSTALL_DIR,
#   CARGO_DIST_FORCE_INSTALL_DIR, NYXID_CLI_NO_MODIFY_PATH (set for staging);
#   curl/cargo/rustup retain their standard network environment settings.
# External endpoints called: github.com and release-assets.githubusercontent.com
#   (prebuilt installer/assets), sh.rustup.rs and static.rust-lang.org (fallback
#   Rust installer/toolchain), github.com, crates.io and static.crates.io
#   (fallback cargo source/dependencies)
# Local files read: shell RC files (~/.zshrc, ~/.bashrc, etc.)
# Local files written: shell RC files (adds the active binary directory),
#   NYXID_ACTIVE_SYMLINK (default ~/.local/bin/nyxid),
#   NYXID_INSTALL_ROOT/vX.Y.Z/nyxid (default $XDG_DATA_HOME/nyxid/versions
#   or ~/.local/share/nyxid/versions), private temporary staging files;
#   source fallback may install Rust under CARGO_HOME (default ~/.cargo).
#
# NyxID CLI installer -- prefers the prebuilt cargo-dist binary installer and
# falls back to cargo install only when no prebuilt installer can be obtained/run.
set -euo pipefail

REPO="https://github.com/ChronoAIProject/NyxID"
INSTALLER_URL="https://github.com/ChronoAIProject/NyxID/releases/latest/download/nyxid-cli-installer.sh"
LOCAL_BIN="$HOME/.local/bin"
ACTIVE_NYXID="${NYXID_ACTIVE_SYMLINK:-$LOCAL_BIN/nyxid}"
VERSIONS_ROOT="${NYXID_INSTALL_ROOT:-${XDG_DATA_HOME:-$HOME/.local/share}/nyxid/versions}"
CARGO_HOME_DIR="${CARGO_HOME:-$HOME/.cargo}"
CARGO_BIN="$CARGO_HOME_DIR/bin"
CARGO_ENV="$CARGO_HOME_DIR/env"
# Symlink targets must remain valid when invoked from another directory.
case "$ACTIVE_NYXID" in /*) ;; *) ACTIVE_NYXID="$PWD/$ACTIVE_NYXID" ;; esac
case "$VERSIONS_ROOT" in /*) ;; *) VERSIONS_ROOT="$PWD/$VERSIONS_ROOT" ;; esac
ACTIVE_DIR="$(dirname "$ACTIVE_NYXID")"
STAGING_DIR=""
TMP_LINK=""
TMP_BINARY=""
cleanup() {
  [ -z "$STAGING_DIR" ] || rm -rf -- "$STAGING_DIR"
  [ -z "$TMP_LINK" ] || rm -f -- "$TMP_LINK"
  [ -z "$TMP_BINARY" ] || rm -f -- "$TMP_BINARY"
  return 0
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

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

  grep -Fq -- "$ACTIVE_DIR" "$rc_file" 2>/dev/null && return 0
  if [ "$ACTIVE_DIR" = "$LOCAL_BIN" ]; then
    grep -Eq '(\$HOME|\$\{HOME\}|~)/\.local/bin|fish_add_path.*\.local/bin' "$rc_file" 2>/dev/null && return 0
  fi
  return 1
}

ensure_active_path() {
  local rc_file shell_name escaped_dir
  rc_file="$(detect_shell_rc)"
  shell_name="$(basename "${SHELL:-/bin/sh}")"

  if path_in_rc "$rc_file"; then
    info "PATH already configured in $rc_file"
    return
  fi

  info "Adding $ACTIVE_DIR to PATH in $rc_file..."
  mkdir -p "$(dirname "$rc_file")"
  # Double-quoted literal path, safe even with shell metacharacters in HOME.
  escaped_dir="$(printf '%s' "$ACTIVE_DIR" | sed 's/[\\"$`]/\\&/g')"
  {
    echo ""
    echo "# NyxID CLI"
    if [ "$shell_name" = "fish" ]; then
      printf 'fish_add_path "%s"\n' "$escaped_dir"
    else
      printf 'export PATH="%s:$PATH"\n' "$escaped_dir"
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
  info "Installing NyxID CLI prebuilt binary..."
  if ! curl --proto '=https' --tlsv1.2 -fsSL "$INSTALLER_URL" -o "$STAGING_DIR/installer.sh"; then
    warn "prebuilt installer download failed"
    return 1
  fi
  # cargo-dist 0.30.0: INSTALL_DIR / FORCE_INSTALL_DIR precede UNMANAGED_INSTALL.
  # Unmanaged mode installs flat and writes neither shell RC files nor receipts.
  if ! NYXID_CLI_INSTALL_DIR= CARGO_DIST_FORCE_INSTALL_DIR= \
      NYXID_CLI_UNMANAGED_INSTALL="$STAGING_DIR/prebuilt" \
      NYXID_CLI_NO_MODIFY_PATH=1 sh "$STAGING_DIR/installer.sh"; then
    warn "prebuilt installer failed"
    return 1
  fi
  install_versioned_binary "$STAGING_DIR/prebuilt/nyxid"
  info "NyxID CLI installed at $ACTIVE_NYXID"
}

is_linux_arm64() {
  case "$(uname -s):$(uname -m)" in
    Linux:aarch64 | Linux:arm64)
      return 0
      ;;
    *)
      return 1
      ;;
  esac
}

compiler_is_clang() {
  local compiler="$1"
  case "$compiler" in
    *clang*)
      return 0
      ;;
  esac

  "$compiler" --version 2>/dev/null | grep -qi 'clang'
}

compiler_major_version() {
  local compiler="$1" version
  version="$("$compiler" -dumpfullversion -dumpversion 2>/dev/null | head -n 1 || true)"
  case "$version" in
    [0-9]*)
      printf '%s\n' "${version%%.*}"
      ;;
    *)
      return 1
      ;;
  esac
}

compiler_is_affected_gcc() {
  local compiler="$1" major
  if compiler_is_clang "$compiler"; then
    return 1
  fi

  major="$(compiler_major_version "$compiler" || true)"
  [ -n "$major" ] && [ "$major" -eq 9 ]
}

source_build_clang_help() {
  printf 'Linux arm64 source builds can fail while compiling aws-lc-sys with affected GCC 9.x versions (gcc#95189). Install clang and retry, or run: CC=clang cargo install --git %s nyxid-cli --force --locked\n' "$REPO"
}

configure_source_build_compiler() {
  local compiler
  if ! is_linux_arm64; then
    return 0
  fi

  if [ -n "${CC:-}" ]; then
    if compiler_is_affected_gcc "$CC"; then
      fail "$(source_build_clang_help)"
    fi
    return 0
  fi

  if command -v clang >/dev/null 2>&1; then
    export CC=clang
    info "Using CC=clang for Linux arm64 source build to avoid the aws-lc-sys GCC compiler guard."
    return 0
  fi

  for compiler in gcc cc; do
    if command -v "$compiler" >/dev/null 2>&1 && compiler_is_affected_gcc "$compiler"; then
      fail "$(source_build_clang_help)"
    fi
  done

  warn "clang was not found. If aws-lc-sys fails with the gcc#95189 memcmp compiler-bug guard, install clang and retry with CC=clang."
}

cargo_log_mentions_aws_lc_gcc_guard() {
  local cargo_log="$1"
  grep -Eiq 'aws-lc-sys' "$cargo_log" \
    && grep -Eiq 'gcc#95189|memcmp|compiler[- ]bug' "$cargo_log"
}

install_versioned_binary() {
  local binary="$1" output raw_version version version_dir versioned_bin
  [ -f "$binary" ] && [ -x "$binary" ] || fail "installer succeeded but staged binary is missing or not executable: $binary"
  output="$("$binary" --version 2>/dev/null)" || fail "staged binary failed to run: $binary --version"
  raw_version="$(printf '%s\n' "$output" | sed -nE 's/^nyxid v?([0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.-]+)?)$/\1/p')"
  [ -n "$raw_version" ] || fail "could not determine staged nyxid version: $binary --version"
  version="v$raw_version"
  version_dir="$VERSIONS_ROOT/$version"
  versioned_bin="$version_dir/nyxid"

  mkdir -p "$version_dir" "$ACTIVE_DIR" || fail "could not create install directories: $version_dir / $ACTIVE_DIR"
  [ ! -d "$ACTIVE_NYXID" ] || fail "active binary path is a directory: $ACTIVE_NYXID"
  [ ! -d "$versioned_bin" ] || fail "versioned binary path is a directory: $versioned_bin"
  TMP_BINARY="$(mktemp "$version_dir/.nyxid.XXXXXX")" || fail "could not stage versioned binary in $version_dir"
  install -m 755 "$binary" "$TMP_BINARY" || fail "could not copy staged binary into $version_dir"
  mv -f "$TMP_BINARY" "$versioned_bin" || fail "could not activate versioned binary: $versioned_bin"
  TMP_BINARY=""
  # Reserve a unique name in the same directory for an atomic symlink rename.
  TMP_LINK="$(mktemp "$ACTIVE_DIR/.nyxid-link.XXXXXX")" || fail "could not stage active symlink in $ACTIVE_DIR"
  rm -f "$TMP_LINK" || fail "could not prepare active symlink: $TMP_LINK"
  ln -s "$versioned_bin" "$TMP_LINK" || fail "could not create active symlink: $TMP_LINK"
  mv -f "$TMP_LINK" "$ACTIVE_NYXID" || fail "could not replace active binary: $ACTIVE_NYXID"
  TMP_LINK=""
  info "Versioned install: $versioned_bin"
}

install_from_source() {
  local cargo_log cargo_status
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

  configure_source_build_compiler

  cargo_log="$STAGING_DIR/cargo.log"
  if cargo install --git "$REPO" nyxid-cli --force --locked --root "$STAGING_DIR/source" 2>&1 | tee "$cargo_log"; then
    rm -f "$cargo_log"
  else
    cargo_status=$?
    if cargo_log_mentions_aws_lc_gcc_guard "$cargo_log"; then
      rm -f "$cargo_log"
      fail "$(source_build_clang_help)"
    fi
    rm -f "$cargo_log"
    fail "cargo install failed with exit code $cargo_status"
  fi

  install_versioned_binary "$STAGING_DIR/source/bin/nyxid"
  info "NyxID CLI installed at $ACTIVE_NYXID"
}

# ---------------------------------------------------------------------------
# Install
# ---------------------------------------------------------------------------

STAGING_DIR="$(mktemp -d "${TMPDIR:-/tmp}/nyxid-install.XXXXXX")" || fail "could not create private staging directory"

if prebuilt_target_supported; then
  if ! install_prebuilt; then
    warn "No usable prebuilt binary was available for this host; using source fallback."
    install_from_source
  fi
else
  warn "No prebuilt NyxID CLI binary is published for $(uname -s)/$(uname -m)."
  install_from_source
fi

ensure_active_path

# ---------------------------------------------------------------------------
# Verify
# ---------------------------------------------------------------------------

[ -x "$ACTIVE_NYXID" ] || fail "active nyxid binary is missing: $ACTIVE_NYXID"
VERIFIED_VERSION="$("$ACTIVE_NYXID" --version)" || fail "active binary failed verification: $ACTIVE_NYXID --version"
info "Verified: $VERIFIED_VERSION"

info ""
info "Installation complete!"
