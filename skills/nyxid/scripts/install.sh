#!/usr/bin/env bash
# SECURITY MANIFEST:
# Environment variables accessed: HOME, SHELL, PATH, CARGO_HOME
# External endpoints called: sh.rustup.rs (Rust installer), github.com (cargo install)
# Local files read: shell RC files (~/.zshrc, ~/.bashrc, etc.)
# Local files written: shell RC files (appends cargo PATH if missing), ~/.local/bin/nyxid (symlink)
#
# NyxID CLI installer -- handles Rust toolchain, CLI binary, and PATH setup.
# Designed for non-technical users who may not have ~/.cargo/bin in PATH.
set -euo pipefail

REPO="https://github.com/ChronoAIProject/NyxID"
CARGO_HOME_DIR="${CARGO_HOME:-$HOME/.cargo}"
CARGO_BIN="$CARGO_HOME_DIR/bin"
CARGO_ENV="$CARGO_HOME_DIR/env"

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

info()  { printf '  %s\n' "$*" >&2; }
warn()  { printf '  [warn] %s\n' "$*" >&2; }
fail()  { printf '  [error] %s\n' "$*" >&2; exit 1; }

# Detect the user's shell RC file.
detect_shell_rc() {
  local shell_name
  shell_name="$(basename "${SHELL:-/bin/sh}")"

  case "$shell_name" in
    zsh)
      echo "$HOME/.zshrc"
      ;;
    bash)
      # macOS bash sessions are commonly login shells; Linux terminals usually
      # source .bashrc for new interactive sessions.
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

# Check if a file already references cargo PATH setup.
cargo_in_rc() {
  local rc_file="$1"
  [ -f "$rc_file" ] || return 1

  grep -Fq "$CARGO_BIN" "$rc_file" 2>/dev/null && return 0
  grep -Fq "$CARGO_ENV" "$rc_file" 2>/dev/null && return 0
  grep -Eq '(\$HOME|\$\{HOME\}|~)/\.cargo/(bin|env)|\.cargo/(bin|env)|fish_add_path' "$rc_file" 2>/dev/null
}

# ---------------------------------------------------------------------------
# Step 1: Ensure Rust / Cargo is available
# ---------------------------------------------------------------------------

if command -v cargo &>/dev/null; then
  info "Rust toolchain already installed ($(cargo --version))"
else
  info "Rust toolchain not found -- installing via rustup..."
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
  info "Rust installed successfully."
fi

# Source cargo env for the current session (needed even if cargo was found,
# because PATH might not include ~/.cargo/bin in this shell)
if [ -f "$CARGO_ENV" ]; then
  # shellcheck disable=SC1090
  . "$CARGO_ENV"
else
  export PATH="$CARGO_BIN:$PATH"
fi

# Verify cargo is now reachable
if ! command -v cargo &>/dev/null; then
  fail "cargo still not found after setup. Please add $CARGO_BIN to your PATH manually."
fi

# ---------------------------------------------------------------------------
# Step 2: Install the NyxID CLI
# ---------------------------------------------------------------------------

info "Installing NyxID CLI..."
# --force ensures we always rebuild from the latest source, even if the
# version string in Cargo.toml hasn't changed (git commits may add features
# like WebSocket proxy support without a version bump).
cargo install --git "$REPO" nyxid-cli --locked --force
info "NyxID CLI installed at $CARGO_BIN/nyxid"

# ---------------------------------------------------------------------------
# Step 3: Symlink into ~/.local/bin (required by OpenClaw and XDG-aware tools)
# ---------------------------------------------------------------------------

LOCAL_BIN="$HOME/.local/bin"

if [ -x "$CARGO_BIN/nyxid" ]; then
  mkdir -p "$LOCAL_BIN"
  ln -sf "$CARGO_BIN/nyxid" "$LOCAL_BIN/nyxid"
  info "Symlinked $LOCAL_BIN/nyxid -> $CARGO_BIN/nyxid"
else
  warn "nyxid binary not found in $CARGO_BIN -- skipping ~/.local/bin symlink"
fi

# ---------------------------------------------------------------------------
# Step 4: Ensure cargo bin is in PATH for future shell sessions
# ---------------------------------------------------------------------------

RC_FILE="$(detect_shell_rc)"
SHELL_NAME="$(basename "${SHELL:-/bin/sh}")"

if cargo_in_rc "$RC_FILE"; then
  info "PATH already configured in $RC_FILE"
else
  info "Adding cargo to PATH in $RC_FILE..."
  mkdir -p "$(dirname "$RC_FILE")"

  {
    echo ""
    echo "# Cargo (Rust package manager) -- added by NyxID installer"
    if [ "$SHELL_NAME" = "fish" ]; then
      printf 'fish_add_path "%s"\n' "$CARGO_BIN"
    elif [ -f "$CARGO_ENV" ]; then
      printf '. "%s"\n' "$CARGO_ENV"
    else
      printf 'export PATH="%s:$PATH"\n' "$CARGO_BIN"
    fi
  } >> "$RC_FILE"

  info "Done -- $RC_FILE updated."
  info "Open a new terminal or run: source $RC_FILE"
fi

# ---------------------------------------------------------------------------
# Step 5: Verify
# ---------------------------------------------------------------------------

if command -v nyxid &>/dev/null; then
  info "Verified: $(nyxid --version 2>/dev/null || echo 'nyxid is available')"
elif [ -x "$CARGO_BIN/nyxid" ]; then
  info "Installed at $CARGO_BIN/nyxid (will be in PATH after opening a new terminal)"
else
  warn "nyxid binary not found -- installation may have failed"
  exit 1
fi

info ""
info "Installation complete!"
