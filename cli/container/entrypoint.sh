#!/bin/sh
set -eu
umask 077
ulimit -c 0
# The setup page passes the owner's capability choices as image arguments.
# A plain docker run opts into the image's documented machine defaults.
if [ "$#" -eq 0 ]; then set -- --machine --computer; fi
for MACHINE_OPTION in "$@"; do
    case "$MACHINE_OPTION" in
        --machine|--shell|--files|--computer) ;;
        *) printf '%s\n' 'Choose --shell, --files, --computer or --machine.' >&2; exit 2 ;;
    esac
done
unset MACHINE_OPTION
MACHINE_STATE=/var/lib/nyxid-machine
mkdir -p "$MACHINE_STATE/node" "$MACHINE_STATE/desktop" /workspace /tmp/.X11-unix
chmod 1777 /tmp/.X11-unix
chmod 0711 "$MACHINE_STATE" "$MACHINE_STATE/desktop"
chmod 0700 "$MACHINE_STATE/node"
chown agent:agent /workspace
# X access belongs only to the browser user. Never use Xvfb -ac.
MACHINE_COOKIE=$(openssl rand -hex 16)
printf 'add :99 MIT-MAGIC-COOKIE-1 %s\n' "$MACHINE_COOKIE" | xauth -f "$XAUTHORITY" source -
unset MACHINE_COOKIE
chown browser:browser "$XAUTHORITY"
chmod 0600 "$XAUTHORITY"
runuser -u browser -- env -i PATH=/usr/bin:/bin HOME=/home/browser DISPLAY=:99 XAUTHORITY="$XAUTHORITY" \
    Xvfb :99 -screen 0 1280x800x24 -nolisten tcp -auth "$XAUTHORITY" &
MACHINE_DISPLAY_PID=$!
trap 'kill "$MACHINE_DISPLAY_PID" 2>/dev/null || true' EXIT
# Openbox waits for the display without exposing Xauthority to command children.
runuser -u browser -- env -i PATH=/usr/bin:/bin HOME=/home/browser DISPLAY=:99 XAUTHORITY="$XAUTHORITY" \
    sh -c 'until xdpyinfo >/dev/null 2>&1; do sleep 0.1; done; exec openbox' >/dev/null 2>&1 &
if [ -f "$MACHINE_STATE/node/config.toml" ]; then
    unset NYXID_NODE_TOKEN
else
    nyxid node setup --container "$@" --computer-mode unrestricted \
        --cua-driver /opt/nyxid/cua/cua-driver --root /workspace --no-daemon --config "$MACHINE_STATE/node"
    unset NYXID_NODE_TOKEN
fi
exec nyxid node start --config "$MACHINE_STATE/node"
