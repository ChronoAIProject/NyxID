#!/usr/bin/env bash
set -euo pipefail

# Hosted Ubuntu mirrors can stall during index acquisition. Retry only package/index
# acquisition; the calling step also bounds locks and installation scripts.
apt_options=(
  -o Acquire::Retries=2
  -o Acquire::http::Timeout=20
  -o Acquire::https::Timeout=20
  -o APT::Update::Error-Mode=any
)

sudo -n env DEBIAN_FRONTEND=noninteractive apt-get "${apt_options[@]}" update
sudo -n env DEBIAN_FRONTEND=noninteractive apt-get "${apt_options[@]}" install -y libdbus-1-dev pkg-config
