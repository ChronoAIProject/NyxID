#!/bin/sh
# The reviewed OS-package contract is recorded in docs/BUILD_SECURITY.md.
# No executable package commands are accepted from the generated dist matrix.
set -eu
if [ "$(uname -s)" != Linux ]; then exit 0; fi
if [ "$(id -u)" = 0 ]; then
    apt-get update
    DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends clang libdbus-1-dev pkg-config python3
else
    sudo apt-get update
    sudo env DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends clang libdbus-1-dev pkg-config python3
fi
