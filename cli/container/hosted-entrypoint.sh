#!/bin/sh
set -eu
export NYXID_MACHINE_CONTAINER=1
umask 077
ulimit -c 0
MACHINE_STATE=/var/lib/nyxid-machine
mkdir -p "$MACHINE_STATE/node" /workspace /home/agent
chmod 0711 "$MACHINE_STATE"
chmod 0700 "$MACHINE_STATE/node"
# exec preserves this PID; the container's Tini reaps orphaned descendants and
# exits with the Node. Kernel PID-namespace teardown then kills every remaining
# browser, desktop, driver and command descendant, including new sessions.
printf '%s\n' "$$" > "$MACHINE_STATE/node/node.pid"

if [ "${NYXID_MACHINE_MANAGED:-${NYXID_MANAGED_KUBERNETES:-0}}" = "1" ]; then
    [ "$#" -eq 0 ] || { echo 'Managed Machine capabilities come from sealed bootstrap.' >&2; exit 2; }
    [ -n "${NYXID_MANAGED_BOOTSTRAP_URL:-}" ] || { echo 'Managed Machine requires a bootstrap endpoint.' >&2; exit 2; }
    unset NYXID_NODE_TOKEN
    if [ "${NYXID_MANAGED_KUBERNETES:-0}" = "1" ]; then
        # The same retained PVC is mounted at its protected root and at only
        # its legacy subdirectory. Never place node secrets below this root.
        mountpoint -q /var/lib/nyxid-contexts || { echo 'Managed Machine requires its retained context volume.' >&2; exit 2; }
        python3 - <<'STORAGE'
import os, stat
root='/var/lib/nyxid-contexts'
volume=os.lstat(root)
legacy=os.lstat(root+'/legacy')
workspace=os.lstat('/workspace')
assert stat.S_ISDIR(volume.st_mode) and volume.st_uid == 0 and not volume.st_mode & 0o022, 'Invalid retained volume root'
assert stat.S_ISDIR(legacy.st_mode) and (legacy.st_dev,legacy.st_ino)==(workspace.st_dev,workspace.st_ino), 'Workspace must be the retained legacy subdirectory'
STORAGE
        # Kubernetes' EmptyDir hides the image's /home permissions. Establish
        # the supervisor-owned parent before starting any worker or desktop.
        chown root:root /home
        chmod 0755 /home
        chown root:agent /workspace
        chmod 0770 /workspace
        chown root:root /home/agent
        chmod 0700 /home/agent
        chown agent:agent /home/agent
        exec nyxid node managed-start --kubernetes --bootstrap-url "$NYXID_MANAGED_BOOTSTRAP_URL" --config "$MACHINE_STATE/node"
    fi
    [ -n "${NYXID_MANAGED_TICKET_FILE:-}" ] || { echo 'Managed Machine requires a protected bootstrap ticket.' >&2; exit 2; }
    exec nyxid node managed-start --ticket-file "$NYXID_MANAGED_TICKET_FILE" --bootstrap-url "$NYXID_MANAGED_BOOTSTRAP_URL" --config "$MACHINE_STATE/node"
fi

mkdir -p /var/lib/nyxid-machine-update
chmod 0700 /var/lib/nyxid-machine-update
# Preserve the existing self-hosted setup choices and defaults.
if [ "$#" -eq 0 ]; then set -- --machine --computer; fi
MACHINE_DESKTOP_ENABLED=0
for MACHINE_OPTION in "$@"; do
    case "$MACHINE_OPTION" in
        --machine|--shell|--files) ;;
        --browser|--computer) MACHINE_DESKTOP_ENABLED=1 ;;
        *) printf '%s\n' 'Choose --shell, --files, --browser, --computer or --machine.' >&2; exit 2 ;;
    esac
done
unset MACHINE_OPTION
if [ -e "$MACHINE_STATE/node/machine-contexts.json" ]; then
    chown root:agent /workspace
    chmod 0770 /workspace
else
    chown agent:agent /workspace
fi
if [ -f "$MACHINE_STATE/node/config.toml" ]; then
    MACHINE_DESKTOP_ENABLED=$(python3 - "$MACHINE_STATE/node/config.toml" <<'CONFIG'
import sys,tomllib
with open(sys.argv[1], 'rb') as stream:
    machine=tomllib.load(stream).get('machine',{})
print(int(bool(machine.get('computer') or machine.get('browser', machine.get('computer')))))
CONFIG
)
fi
if [ "$MACHINE_DESKTOP_ENABLED" = "1" ]; then
    /usr/local/bin/nyxid-machine-desktop-start
fi
unset MACHINE_DESKTOP_ENABLED
if [ ! -f "$MACHINE_STATE/node/config.toml" ]; then
    nyxid node setup --container "$@" --computer-mode unrestricted \
        --cua-driver /opt/nyxid/cua/cua-driver --root /workspace --no-daemon --config "$MACHINE_STATE/node"
fi
unset NYXID_NODE_TOKEN
exec nyxid node start --config "$MACHINE_STATE/node"
