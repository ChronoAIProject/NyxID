#!/usr/bin/env bash
# node-docker.sh - Manage NyxID node agents via Docker
#
# Usage:
#   ./scripts/node-docker.sh start [profile]        Start a node agent container
#   ./scripts/node-docker.sh stop [profile]         Stop a node agent container
#   ./scripts/node-docker.sh restart [profile]      Restart a node agent container
#   ./scripts/node-docker.sh status [profile]       Show container status
#   ./scripts/node-docker.sh logs [profile]         Tail container logs
#   ./scripts/node-docker.sh build                  Build the node agent image
#
# Profile is optional. Without it, uses the default config at ~/.nyxid-node/.
# With a profile name, uses ~/.nyxid-node/profiles/<name>/.
#
# Pre-requisite: register the node on the host first:
#   nyxid node register --token <token> --url <ws-url> [--profile <name>]

set -euo pipefail

IMAGE="nyxid-node:latest"
BASE_DIR="${HOME}/.nyxid-node"

resolve_config_dir() {
    local profile="${1:-}"
    if [ -z "$profile" ] || [ "$profile" = "default" ]; then
        echo "$BASE_DIR"
    else
        echo "$BASE_DIR/profiles/$profile"
    fi
}

container_name() {
    local profile="${1:-}"
    if [ -z "$profile" ] || [ "$profile" = "default" ]; then
        echo "nyxid-node"
    else
        echo "nyxid-node-${profile}"
    fi
}

cmd_build() {
    local script_dir
    script_dir="$(cd "$(dirname "$0")" && pwd)"
    local project_root="$script_dir/.."
    echo "Building node agent image..."
    docker build -f "$project_root/cli/Dockerfile.node" -t "$IMAGE" "$project_root"
}

cmd_start() {
    local profile="${1:-}"
    local config_dir
    config_dir="$(resolve_config_dir "$profile")"
    local name
    name="$(container_name "$profile")"

    # Check if image exists, build if not
    if ! docker image inspect "$IMAGE" >/dev/null 2>&1; then
        cmd_build
    fi

    # Build env args for auto-registration (entrypoint handles the logic)
    local env_args=()
    if [ -n "${NYXID_NODE_TOKEN:-}" ]; then
        env_args+=(-e "NYXID_NODE_TOKEN=$NYXID_NODE_TOKEN")
    fi
    if [ -n "${NYXID_NODE_URL:-}" ]; then
        env_args+=(-e "NYXID_NODE_URL=$NYXID_NODE_URL")
    fi

    if [ ! -f "$config_dir/config.toml" ] && [ -z "${NYXID_NODE_TOKEN:-}" ]; then
        echo "Error: $config_dir/config.toml not found."
        echo "Either register the node first or pass NYXID_NODE_TOKEN:"
        echo "  NYXID_NODE_TOKEN=nyx_nreg_... NYXID_NODE_URL=wss://... $0 start ${profile:-}"
        exit 1
    fi

    mkdir -p "$config_dir"

    echo "Starting node agent: $name (config: $config_dir)"
    docker run -d \
        --name "$name" \
        --restart unless-stopped \
        --user "$(id -u):$(id -g)" \
        -v "$config_dir:/app/config:rw" \
        "${env_args[@]}" \
        "$IMAGE"

    echo "Container $name started."
    echo "  Logs:   docker logs -f $name"
    echo "  Stop:   $0 stop ${profile:-}"
    echo "  Status: $0 status ${profile:-}"
}

cmd_stop() {
    local profile="${1:-}"
    local name
    name="$(container_name "$profile")"
    echo "Stopping $name..."
    docker stop "$name" 2>/dev/null && docker rm "$name" 2>/dev/null || true
    echo "Stopped."
}

cmd_restart() {
    local profile="${1:-}"
    cmd_stop "$profile"
    cmd_start "$profile"
}

cmd_status() {
    local profile="${1:-}"
    local name
    name="$(container_name "$profile")"
    if docker ps --format '{{.Names}}' | grep -q "^${name}$"; then
        echo "$name: running"
        docker ps --filter "name=^${name}$" --format "  ID: {{.ID}}  Up: {{.Status}}  Image: {{.Image}}"
    elif docker ps -a --format '{{.Names}}' | grep -q "^${name}$"; then
        echo "$name: stopped"
        docker ps -a --filter "name=^${name}$" --format "  ID: {{.ID}}  Status: {{.Status}}"
    else
        echo "$name: not found"
    fi
}

cmd_logs() {
    local profile="${1:-}"
    local name
    name="$(container_name "$profile")"
    docker logs -f "$name"
}

case "${1:-help}" in
    build)   cmd_build ;;
    start)   cmd_start "${2:-}" ;;
    stop)    cmd_stop "${2:-}" ;;
    restart) cmd_restart "${2:-}" ;;
    status)  cmd_status "${2:-}" ;;
    logs)    cmd_logs "${2:-}" ;;
    *)
        echo "Usage: $0 {build|start|stop|restart|status|logs} [profile]"
        echo ""
        echo "Commands:"
        echo "  build              Build the node agent Docker image"
        echo "  start [profile]    Start a node agent container"
        echo "  stop [profile]     Stop and remove a node agent container"
        echo "  restart [profile]  Restart a node agent container"
        echo "  status [profile]   Show container status"
        echo "  logs [profile]     Tail container logs"
        echo ""
        echo "Examples:"
        echo "  $0 start                    # Default node"
        echo "  $0 start coding-agent       # Profile-specific node"
        echo "  $0 status                   # Check default node"
        echo "  $0 logs research-agent      # Tail profile logs"
        exit 1
        ;;
esac
