#!/usr/bin/env bash
set -euo pipefail

# A measured backend test compile used 2.4 GiB of the runner's 3 GiB swap.
# Preserve compiler concurrency and provide headroom for short memory peaks.
minimum_swap_kib=$((8 * 1024 * 1024))
current_swap_kib="$(awk '$1 == "SwapTotal:" { print $2 }' /proc/meminfo)"
[[ "$current_swap_kib" =~ ^[0-9]+$ ]]
echo 'Backend memory capacity before preparation:'
free -m
if ((current_swap_kib >= minimum_swap_kib)); then
  free -m
  exit 0
fi

additional_mib=$(((minimum_swap_kib - current_swap_kib + 1023) / 1024 + 1))
available_disk_kib="$(df -Pk "${RUNNER_TEMP:?}" | awk 'END { print $4 }')"
[[ "$available_disk_kib" =~ ^[0-9]+$ ]]
if ((available_disk_kib < additional_mib * 1024 + 8 * 1024 * 1024)); then
  echo 'Insufficient disk to provide backend swap reserve and retain 8 GiB free.' >&2
  exit 1
fi

swap_file="$(mktemp "$RUNNER_TEMP/nyxid-ci-swap.XXXXXX")"
activated=false
cleanup() {
  if [[ "$activated" != true ]]; then
    rm -f "$swap_file"
  fi
}
trap cleanup EXIT
chmod 600 "$swap_file"
fallocate -l "${additional_mib}M" "$swap_file"
sudo -n mkswap "$swap_file" >/dev/null
sudo -n swapon "$swap_file"
activated=true

actual_swap_kib="$(awk '$1 == "SwapTotal:" { print $2 }' /proc/meminfo)"
if ((actual_swap_kib < minimum_swap_kib)); then
  echo 'Backend swap reserve was not established.' >&2
  exit 1
fi
echo 'Backend runner has at least 8 GiB total swap; concurrency is unchanged.'
free -m
