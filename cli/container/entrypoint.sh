#!/bin/sh
set -eu
export NYXID_MACHINE_CONTAINER=1
mkdir -p /var/lib/nyxid-machine-update
chmod 0700 /var/lib/nyxid-machine-update
umask 077
ulimit -c 0
# The setup page passes the owner's capability choices as image arguments.
# A plain docker run opts into the image's documented machine defaults.
if [ "$#" -eq 0 ]; then set -- --machine --computer; fi
for MACHINE_OPTION in "$@"; do
    case "$MACHINE_OPTION" in
        --machine|--shell|--files|--browser|--computer) ;;
        *) printf '%s\n' 'Choose --shell, --files, --browser, --computer or --machine.' >&2; exit 2 ;;
    esac
done
unset MACHINE_OPTION
MACHINE_STATE=/var/lib/nyxid-machine
mkdir -p "$MACHINE_STATE/node" "$MACHINE_STATE/desktop" /workspace /tmp/.X11-unix
chmod 1777 /tmp/.X11-unix
chmod 0711 "$MACHINE_STATE" "$MACHINE_STATE/desktop"
chmod 0700 "$MACHINE_STATE/node"
chown agent:agent /workspace
# Each browser has an independent display and authentication cookie.
export NYXID_DEV_DISPLAY=:100
export NYXID_DEV_XAUTHORITY=/home/devbrowser/.Xauthority
for MACHINE_DESKTOP in secure dev; do
    if [ "$MACHINE_DESKTOP" = secure ]; then
        MACHINE_USER=browser
        MACHINE_DISPLAY=:99
        MACHINE_AUTH="$XAUTHORITY"
    else
        MACHINE_USER=devbrowser
        MACHINE_DISPLAY="$NYXID_DEV_DISPLAY"
        MACHINE_AUTH="$NYXID_DEV_XAUTHORITY"
    fi
    MACHINE_COOKIE=$(openssl rand -hex 16)
    printf 'add %s MIT-MAGIC-COOKIE-1 %s\n' "$MACHINE_DISPLAY" "$MACHINE_COOKIE" | xauth -f "$MACHINE_AUTH" source -
    unset MACHINE_COOKIE
    chown "$MACHINE_USER:$MACHINE_USER" "$MACHINE_AUTH"
    chmod 0600 "$MACHINE_AUTH"
    runuser -u "$MACHINE_USER" -- env -i PATH=/usr/bin:/bin HOME="/home/$MACHINE_USER" DISPLAY="$MACHINE_DISPLAY" XAUTHORITY="$MACHINE_AUTH" \
        Xvfb "$MACHINE_DISPLAY" -screen 0 1280x800x24 -nolisten tcp -auth "$MACHINE_AUTH" &
    if [ "$MACHINE_DESKTOP" = dev ]; then
        runuser -u devbrowser -- env -i PATH=/usr/bin:/bin HOME=/home/devbrowser DISPLAY="$MACHINE_DISPLAY" XAUTHORITY="$MACHINE_AUTH" \
            sh -c 'until xdpyinfo >/dev/null 2>&1; do sleep 0.1; done; exec openbox' >/dev/null 2>&1 &
    fi
done
unset MACHINE_DESKTOP MACHINE_USER MACHINE_DISPLAY MACHINE_AUTH
rm -f /home/browser/.nyxid-desktop/session-bus.address
# Openbox waits for the display without exposing Xauthority to command children.
runuser -u browser -- env -i PATH=/usr/bin:/bin HOME=/home/browser DISPLAY=:99 XAUTHORITY="$XAUTHORITY" \
    /usr/local/bin/nyxid-desktop-session >/dev/null 2>&1 &
# This is the browser user's private D-Bus session, never an agent environment.
MACHINE_SESSION_PID=$!
MACHINE_SESSION_WAIT=0
while [ ! -s /home/browser/.nyxid-desktop/session-bus.address ]; do
    if ! kill -0 "$MACHINE_SESSION_PID" 2>/dev/null || [ "$MACHINE_SESSION_WAIT" -ge 200 ]; then
        printf '%s\n' 'Browser accessibility session failed to start; check D-Bus and at-spi2-core.' >&2
        exit 1
    fi
    MACHINE_SESSION_WAIT=$((MACHINE_SESSION_WAIT + 1))
    sleep 0.1
done
unset MACHINE_SESSION_WAIT
if [ -f "$MACHINE_STATE/node/config.toml" ]; then
    unset NYXID_NODE_TOKEN
else
    nyxid node setup --container "$@" --computer-mode unrestricted \
        --cua-driver /opt/nyxid/cua/cua-driver --root /workspace --no-daemon --config "$MACHINE_STATE/node"
    unset NYXID_NODE_TOKEN
fi
exec nyxid node start --config "$MACHINE_STATE/node"
