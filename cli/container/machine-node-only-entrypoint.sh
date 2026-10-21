#!/bin/sh
set -eu

STATE_DIR=/var/lib/nyxid-machine
NODE_DIR="$STATE_DIR/node"
CONFIG="$NODE_DIR/config.toml"
PID_FILE="$NODE_DIR/node.pid"

umask 077
mkdir -p "$NODE_DIR" /workspace
chmod 0711 "$STATE_DIR"
chmod 0700 "$NODE_DIR"

if [ "${NYXID_MACHINE_MANAGED:-1}" = "1" ]; then
    if [ -z "${NYXID_MANAGED_BOOTSTRAP_URL:-}" ]; then
        echo "Managed Remote Machine startup requires a bootstrap endpoint." >&2
        exit 2
    fi
    if [ "${NYXID_MANAGED_KUBERNETES:-0}" = "1" ]; then
        # Only the root supervisor can read the audience-bound projected Pod
        # token. It is never copied into a command's environment or workspace.
        nyxid node managed-start --kubernetes \
            --bootstrap-url "$NYXID_MANAGED_BOOTSTRAP_URL" --config "$NODE_DIR" &
    elif [ -n "${NYXID_MANAGED_TICKET_FILE:-}" ]; then
        nyxid node managed-start \
            --ticket-file "$NYXID_MANAGED_TICKET_FILE" \
            --bootstrap-url "$NYXID_MANAGED_BOOTSTRAP_URL" --config "$NODE_DIR" &
    else
        echo "Managed Remote Machine startup requires NYXID_MANAGED_TICKET_FILE and NYXID_MANAGED_BOOTSTRAP_URL." >&2
        echo "For a self-hosted node, explicitly set NYXID_MACHINE_MANAGED=0 and provide NYXID_NODE_TOKEN." >&2
        exit 2
    fi
    # Always re-enter managed-start. On an existing volume it validates the
    # provider ticket against the persisted binding before starting the Node;
    # a stale or foreign identity must never fall through to ordinary startup.
    node_pid=$!
    printf '%s\n' "$node_pid" > "$PID_FILE"
    chmod 0600 "$PID_FILE"
    cleanup() {
        kill "$node_pid" 2>/dev/null || true
        wait "$node_pid" 2>/dev/null || true
    }
    trap cleanup INT TERM EXIT
    wait "$node_pid"
    exit $?
else
  chown agent:agent /workspace /home/agent
  if [ ! -s "$CONFIG" ]; then
    # Absence is automatic, never consent to legacy. This browserless image
    # cannot confirm separated contexts; setup explains the required explicit
    # choice before registration. An existing profile retains its stored intent.
    MACHINE_SEPARATION=${NYXID_MACHINE_SEPARATION-automatic}
    case "$MACHINE_SEPARATION" in
        automatic|require-separated|shared-legacy) ;;
        *) echo "NYXID_MACHINE_SEPARATION must be automatic, require-separated or shared-legacy." >&2; exit 2 ;;
    esac
    if [ -z "${NYXID_NODE_TOKEN:-}" ]; then
        echo "NYXID_NODE_TOKEN is required on first start; mount an existing identity or provide a registration token." >&2
        exit 2
    fi
    if [ -z "${NYXID_NODE_URL:-}" ]; then
        echo "NYXID_NODE_URL is required on first start for a remote machine." >&2
        exit 2
    fi
    # This path registers exactly once. The token is never written to the
    # workspace or retained in the environment after setup returns.
    nyxid node setup \
        --token "$NYXID_NODE_TOKEN" \
        --url "$NYXID_NODE_URL" \
        --node-only-machine-preset \
        --separation "$MACHINE_SEPARATION" \
        --container \
        --no-daemon \
        --skip-browser-policy \
        --config "$NODE_DIR"
    unset NYXID_NODE_TOKEN
  fi
fi

# Refuse an accidentally reused desktop/browser profile. This node-only helper is
# shell/files only; no hidden cua/browser capability may be enabled by a stale
# config volume.
if grep -Eq '^(computer|browser)[[:space:]]*=[[:space:]]*true' "$CONFIG"; then
    echo "Refusing a non shell/files node profile in the node-only helper image." >&2
    exit 3
fi

nyxid node start --config "$NODE_DIR" &
node_pid=$!
printf '%s\n' "$node_pid" > "$PID_FILE"
chmod 0600 "$PID_FILE"

cleanup() {
    kill "$node_pid" 2>/dev/null || true
    wait "$node_pid" 2>/dev/null || true
}
trap cleanup INT TERM EXIT
wait "$node_pid"
