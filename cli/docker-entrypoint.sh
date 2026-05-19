#!/bin/sh
set -e

CONFIG_DIR="${NYXID_NODE_CONFIG_DIR:-/app/config}"
CONFIG_FILE="$CONFIG_DIR/config.toml"

# If no config exists but a registration token is provided, register first.
if [ ! -f "$CONFIG_FILE" ] && [ -n "${NYXID_NODE_TOKEN:-}" ]; then
    REGISTER_URL="${NYXID_NODE_URL:-ws://localhost:3001/api/v1/nodes/ws}"
    echo "No config found. Registering node with $REGISTER_URL ..."
    nyxid node register \
        --token "$NYXID_NODE_TOKEN" \
        --url "$REGISTER_URL" \
        --config "$CONFIG_DIR"
    echo "Registration complete."
fi

if [ ! -f "$CONFIG_FILE" ]; then
    echo "Error: No config at $CONFIG_FILE."
    echo "Either mount an existing config or set NYXID_NODE_TOKEN + NYXID_NODE_URL to register."
    exit 1
fi

exec nyxid node start --config "$CONFIG_DIR" "$@"
