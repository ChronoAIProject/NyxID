#!/usr/bin/env bash
set -euo pipefail

swap_record="${RUNNER_TEMP:?}/nyxid-ci-swap-path"
if [[ "${1:-}" == --release && $# == 1 ]]; then
  # Compilation has finished. Return the reserve's disk space before the
  # cache action creates its compressed archive; never touch runner-owned swap.
  if [[ ! -e "$swap_record" ]]; then
    exit 0
  fi
  swap_file="$(cat "$swap_record")"
  if [[ "$swap_file" != "$RUNNER_TEMP"/nyxid-ci-swap.?????? || ! -f "$swap_file" || -L "$swap_file" ]]; then
    echo 'Invalid backend swap record; refusing to release it.' >&2
    exit 1
  fi
  sudo -n swapoff "$swap_file"
  rm -f "$swap_file" "$swap_record"
  echo 'Released the backend compile reserve before cache compression.'
  df -h "$RUNNER_TEMP"
  exit 0
elif (($# != 0)); then
  echo 'usage: prepare-backend-runner.sh [--release]' >&2
  exit 1
fi

# A measured backend test compile used 2.4 GiB of the runner's 3 GiB swap;
# by October 2026 a single nyxid rustc exhausted 16 GiB RAM plus 8 GiB swap,
# and later that month the test-binary compile peaked at 29.7-31.5 GiB of
# RAM plus swap, against 32 GiB with a 16 GiB reserve (main: 28.7 GiB).
# Two subsequent test builds lost their runners with nearly all 16 GiB RAM
# and 24 GiB swap in use. The shutdown cause was not established; reserve
# another 16 GiB for these measured peaks without changing validation.
# Preserve compiler concurrency and provide headroom for short memory peaks.
minimum_swap_kib=$((40 * 1024 * 1024))
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
printf '%s\n' "$swap_file" > "$swap_record"

actual_swap_kib="$(awk '$1 == "SwapTotal:" { print $2 }' /proc/meminfo)"
if ((actual_swap_kib < minimum_swap_kib)); then
  echo 'Backend swap reserve was not established.' >&2
  exit 1
fi
echo 'Backend runner has at least 40 GiB total swap; concurrency is unchanged.'
free -m
