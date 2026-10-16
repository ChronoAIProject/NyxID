#!/bin/sh
set -eu
if [ "${NYXID_MACHINE_MANAGED:-${NYXID_MANAGED_KUBERNETES:-0}}" = "1" ]; then
    # Managed lifecycle uses its own WSS/generation/deadline authority and
    # never initializes the self-hosted updater.
    test -s /var/lib/nyxid-machine/node/config.toml
    test -s /var/lib/nyxid-machine/node/node.pid
    kill -0 "$(cat /var/lib/nyxid-machine/node/node.pid)"
else
    exec nyxid node machine-updater health
fi
