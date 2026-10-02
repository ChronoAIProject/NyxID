#!/bin/sh
# Run as the browser user; the bus, activation services and address are private.
set -eu
umask 077
mkdir -p "$HOME/.nyxid-desktop"
chmod 0700 "$HOME/.nyxid-desktop"
until xdpyinfo >/dev/null 2>&1; do sleep 0.1; done
exec dbus-run-session -- sh -eu -c '
    dbus-send --session --print-reply --dest=org.a11y.Bus /org/a11y/bus org.a11y.Bus.GetAddress >/dev/null
    printf "%s\n" "$DBUS_SESSION_BUS_ADDRESS" > "$HOME/.nyxid-desktop/session-bus.address"
    trap '\''rm -f "$HOME/.nyxid-desktop/session-bus.address"'\'' EXIT
    openbox
'
