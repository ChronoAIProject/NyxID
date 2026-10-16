#!/bin/sh
# Shared by self-hosted setup and sealed managed startup.
set -eu
umask 077
: "${XAUTHORITY:=/var/lib/nyxid-machine/desktop/Xauthority}"
export XAUTHORITY
mkdir -p /var/lib/nyxid-machine/desktop /tmp/.X11-unix /home/browser /home/devbrowser
chmod 1777 /tmp/.X11-unix
chmod 0711 /var/lib/nyxid-machine /var/lib/nyxid-machine/desktop
# CHOWN is available to the supervisor; FOWNER/DAC_OVERRIDE are not. Apply
# modes while the supervisor owns each directory, then hand it to its user.
chown root:root /home/browser /home/devbrowser
chmod 0700 /home/browser /home/devbrowser
chown browser:browser /home/browser
chown devbrowser:devbrowser /home/devbrowser
# Each browser has an independent display and authentication cookie.
export NYXID_DEV_DISPLAY=:100
export NYXID_DEV_XAUTHORITY=/var/lib/nyxid-machine/desktop/dev-Xauthority
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
    # Cookies are temporary. Recreate them without needing DAC override to
    # read a prior browser-owned mode-0600 cookie after a container restart.
    rm -f "$MACHINE_AUTH"
    MACHINE_COOKIE=$(openssl rand -hex 16)
    printf 'add %s MIT-MAGIC-COOKIE-1 %s\n' "$MACHINE_DISPLAY" "$MACHINE_COOKIE" | xauth -f "$MACHINE_AUTH" source -
    unset MACHINE_COOKIE
    chown "root:$MACHINE_USER" "$MACHINE_AUTH"
    chmod 0640 "$MACHINE_AUTH"
    runuser -u "$MACHINE_USER" -- env -i PATH=/usr/bin:/bin HOME="/home/$MACHINE_USER" DISPLAY="$MACHINE_DISPLAY" XAUTHORITY="$MACHINE_AUTH" \
        Xvfb "$MACHINE_DISPLAY" -screen 0 1280x800x24 -nolisten tcp -auth "$MACHINE_AUTH" &
    if [ "$MACHINE_DESKTOP" = dev ]; then
        runuser -u devbrowser -- env -i PATH=/usr/bin:/bin HOME=/home/devbrowser DISPLAY="$MACHINE_DISPLAY" XAUTHORITY="$MACHINE_AUTH" \
            sh -c 'until xdpyinfo >/dev/null 2>&1; do sleep 0.1; done; exec openbox' >/dev/null 2>&1 &
    fi
done
unset MACHINE_DESKTOP MACHINE_USER MACHINE_DISPLAY MACHINE_AUTH
runuser -u browser -- rm -f /home/browser/.nyxid-desktop/session-bus.address
# Openbox waits for the display without exposing Xauthority to command children.
runuser -u browser -- env -i PATH=/usr/bin:/bin HOME=/home/browser DISPLAY=:99 XAUTHORITY="$XAUTHORITY" \
    /usr/local/bin/nyxid-desktop-session >/dev/null 2>&1 &
# This is the browser user's private D-Bus session, never an agent environment.
MACHINE_SESSION_PID=$!
MACHINE_SESSION_WAIT=0
while ! runuser -u browser -- test -s /home/browser/.nyxid-desktop/session-bus.address; do
    if ! kill -0 "$MACHINE_SESSION_PID" 2>/dev/null || [ "$MACHINE_SESSION_WAIT" -ge 200 ]; then
        printf '%s\n' 'Browser accessibility session failed to start; check D-Bus and at-spi2-core.' >&2
        exit 1
    fi
    MACHINE_SESSION_WAIT=$((MACHINE_SESSION_WAIT + 1))
    sleep 0.1
done
runuser -u browser -- cat /home/browser/.nyxid-desktop/session-bus.address > /var/lib/nyxid-machine/desktop/session-bus.address
chmod 0600 /var/lib/nyxid-machine/desktop/session-bus.address
unset MACHINE_SESSION_WAIT
