#!/bin/bash
set -euo pipefail
keyfile=/data/configdb/nyxid-replica-key
if [ ! -s "$keyfile" ]; then
  umask 077
  openssl rand -base64 756 > "$keyfile"
fi
chown mongodb:mongodb "$keyfile"
chmod 400 "$keyfile"
exec /usr/local/bin/docker-entrypoint.sh mongod --replSet nyxid-rs --bind_ip_all --keyFile "$keyfile"
