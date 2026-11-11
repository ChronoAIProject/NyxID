#!/usr/bin/env bash
set -euo pipefail

# Hosted runners' mirror+file sources can stall past acquisition timeouts. Use
# direct, signed Ubuntu repositories for this install without changing the host.
# shellcheck source=/dev/null
source /etc/os-release
if [[ "$ID" != ubuntu || ! "${VERSION_CODENAME:-}" =~ ^[a-z]+$ ]]; then
  echo 'Build dependencies require an Ubuntu release codename.' >&2
  exit 1
fi

case "$(dpkg --print-architecture)" in
  amd64|i386)
    archive=https://archive.ubuntu.com/ubuntu
    security=https://security.ubuntu.com/ubuntu
    ;;
  *)
    archive=https://ports.ubuntu.com/ubuntu-ports
    security=$archive
    ;;
esac

sources_dir="$(mktemp -d)"
trap 'rm -rf "$sources_dir"' EXIT
sources_file="$sources_dir/ubuntu.sources"
cat > "$sources_file" <<EOF
Types: deb
URIs: $archive
Suites: $VERSION_CODENAME $VERSION_CODENAME-updates $VERSION_CODENAME-backports
Components: main restricted universe multiverse
Signed-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg

Types: deb
URIs: $security
Suites: $VERSION_CODENAME-security
Components: main restricted universe multiverse
Signed-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg
EOF
# apt drops privileges to _apt while reading source files.
chmod 755 "$sources_dir"
chmod 644 "$sources_file"

# Retry only package/index acquisition; the calling step also bounds locks and
# installation scripts. Ignore unrelated runner repositories for both commands.
apt_options=(
  -o "Dir::Etc::sourcelist=$sources_file"
  -o Dir::Etc::sourceparts=-
  -o Acquire::Retries=2
  -o Acquire::http::Timeout=20
  -o Acquire::https::Timeout=20
  -o APT::Update::Error-Mode=any
)

sudo -n env DEBIAN_FRONTEND=noninteractive apt-get "${apt_options[@]}" update
sudo -n env DEBIAN_FRONTEND=noninteractive apt-get "${apt_options[@]}" install -y libdbus-1-dev pkg-config ripgrep
