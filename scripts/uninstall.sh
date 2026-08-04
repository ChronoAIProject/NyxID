#!/usr/bin/env bash
# scripts/uninstall.sh -- Remove a local NyxID install so you can re-run the
# self-host quickstart (README.md) from a clean slate.
#
# Quickstart is a first-time install only. To reinstall, run this script,
# then re-paste Step 2.
#
# Removes by default:
#   - Docker containers: nyxid-mongodb, nyxid-mailpit, nyxid-backend, nyxid-frontend
#   - Docker volume:     *_mongodb_data (any compose project name variant)
#   - .env.dev, .env.production, keys/private.pem, keys/public.pem
#
# Preserves:
#   - Docker images (mongo:8.0, ghcr.io/chronoaiproject/nyxid/*) -- no re-pull
#
# Usage:
#   ./scripts/uninstall.sh                 interactive (type "wipe" to confirm)
#   ./scripts/uninstall.sh --yes           non-interactive (CI / repeat testing)
#   ./scripts/uninstall.sh --keep-config   keep .env.dev / .env.production / keys/*.pem
#                                          (useful if you want to preserve your
#                                          existing ENCRYPTION_KEY across reinstall)
#
set -euo pipefail

ASSUME_YES=0
KEEP_CONFIG=0

info() { printf '\033[0;36m[uninstall]\033[0m %s\n' "$*"; }
warn() { printf '\033[0;33m[uninstall]\033[0m %s\n' "$*" >&2; }
fail() { printf '\033[0;31m[uninstall]\033[0m %s\n' "$*" >&2; exit 1; }

usage() {
  cat <<'USAGE'
Usage: scripts/uninstall.sh [OPTIONS]

Remove a local NyxID install so you can re-run the self-host quickstart
(README.md) from a clean slate.

Options:
  -y, --yes             Skip the interactive confirmation (CI / repeat testing).
      --keep-config     Keep .env.dev / .env.production / keys/*.pem.
                        By default these are removed so the reinstall generates
                        a fresh ENCRYPTION_KEY and MONGO_ROOT_PASSWORD. Pass
                        this flag if you want to preserve your existing keys
                        across reinstall (e.g. to keep encrypted DB backups
                        readable).
  -h, --help            Show this help and exit.

Docker images are always preserved so you don't re-download Mongo on every
uninstall.
USAGE
}

parse_args() {
  while [ $# -gt 0 ]; do
    case "$1" in
      -y|--yes)       ASSUME_YES=1 ;;
      --keep-config)  KEEP_CONFIG=1 ;;
      -h|--help)      usage; exit 0 ;;
      *)              fail "Unknown option: $1 (try --help)" ;;
    esac
    shift
  done
}

confirm_or_exit() {
  if [ "$ASSUME_YES" -eq 1 ]; then
    return 0
  fi
  if [ ! -t 0 ]; then
    fail 'stdin is not a TTY; pass --yes to confirm non-interactively.'
  fi

  cat >&2 <<'WARN'

  This will permanently delete:
    - Docker containers: nyxid-mongodb, nyxid-mailpit, nyxid-backend, nyxid-frontend
    - Docker volume:     <project>_mongodb_data
    - All NyxID accounts, encrypted credentials, and audit log entries
      stored in your local MongoDB

WARN

  if [ "$KEEP_CONFIG" -eq 1 ]; then
    cat >&2 <<'EXTRA'
  --keep-config is set: .env.dev, .env.production, and keys/*.pem will be
  preserved. The reinstall will reuse your existing ENCRYPTION_KEY and
  MONGO_ROOT_PASSWORD (needed for encrypted DB backups to stay readable).

EXTRA
  else
    cat >&2 <<'EXTRA'
  Also removing:
    - .env.dev / .env.production / keys/*.pem

  A fresh ENCRYPTION_KEY will be generated on reinstall. Any pre-existing
  encrypted database backup will become unreadable. Pass --keep-config to
  preserve these files.

EXTRA
  fi

  cat >&2 <<'WARN'
  Docker images are preserved (no re-download).

WARN

  printf '  Type "wipe" to confirm: ' >&2
  local reply=""
  read -r reply || true
  if [ "$reply" != "wipe" ]; then
    fail 'Aborted.'
  fi
}

down_compose() {
  # Run `down -v` against both the dev (auto-loaded override) and the
  # documented prod compose flow. Empty stacks no-op; errors are tolerated
  # because the container/volume sweeps below are the real safety net.
  info 'Stopping dev compose stack (if running)...'
  docker compose down -v --remove-orphans >/dev/null 2>&1 || true

  if [ -f .env.production ]; then
    info 'Stopping prod compose stack (if running)...'
    docker compose -f docker-compose.yml -f docker-compose.prod.yml \
      --env-file .env.production down -v --remove-orphans >/dev/null 2>&1 || true
  fi
}

cleanup_orphan_containers() {
  # Belt-and-suspenders: docker-compose.yml hardcodes container_name fields,
  # so a leftover container from a prior run under a different compose project
  # name (renamed clone dir, etc.) survives the `down` above. Sweep by name.
  local c
  for c in nyxid-mongodb nyxid-mailpit nyxid-backend nyxid-frontend; do
    if docker ps -a --format '{{.Names}}' 2>/dev/null | grep -qx "$c"; then
      if docker rm -f "$c" >/dev/null 2>&1; then
        info "Removed orphan container: $c"
      fi
    fi
  done
}

cleanup_orphan_volumes() {
  # Volume name = "${COMPOSE_PROJECT_NAME:-$(basename PWD)}_mongodb_data".
  # Match by exact name only -- NEVER `docker volume prune` (would nuke
  # unrelated projects' dangling volumes on the same host).
  local proj v
  proj="$(basename "$PWD")"
  for v in "${proj}_mongodb_data" "nyxid_mongodb_data" "nyx_mongodb_data"; do
    if docker volume inspect "$v" >/dev/null 2>&1; then
      if docker volume rm "$v" >/dev/null 2>&1; then
        info "Removed volume: $v"
      fi
    fi
  done
}

remove_config_unless_kept() {
  if [ "$KEEP_CONFIG" -eq 1 ]; then
    info 'Keeping .env.dev / .env.production / keys/*.pem (--keep-config).'
    return 0
  fi
  local removed=0
  for f in .env.dev .env.production keys/private.pem keys/public.pem; do
    if [ -f "$f" ] || [ -L "$f" ]; then
      rm -f "$f"
      removed=1
    fi
  done
  if [ "$removed" -eq 1 ]; then
    info 'Removed .env.dev, .env.production, keys/*.pem.'
  fi
}

main() {
  parse_args "$@"
  confirm_or_exit
  down_compose
  cleanup_orphan_containers
  cleanup_orphan_volumes
  remove_config_unless_kept
  info 'Done. Re-run quickstart Step 2 for a fresh install.'
}

main "$@"
