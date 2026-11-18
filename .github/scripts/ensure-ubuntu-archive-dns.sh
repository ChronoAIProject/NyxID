#!/usr/bin/env bash
set -euo pipefail

# Hosted runners resolve through one upstream DNS server. When Canonical's
# authoritative servers are unreachable from it, every *.ubuntu.com lookup
# times out and apt fails on the runner and inside image builds, although the
# archive itself stays reachable. Only in that case, put public resolvers in
# front of the runner's own server for the rest of the job. Package integrity
# does not depend on DNS: apt verifies the archive signatures.
#
# No-op when the lookup works, on non-systemd-resolved hosts, or when the
# fallback cannot be applied; the caller's own step then reports the failure.

if [[ "${NYXID_CI_SKIP_DNS_FALLBACK:-}" == 1 ]]; then
  exit 0
fi

probe_host=archive.ubuntu.com
resolves() { timeout 10 getent ahostsv4 "$probe_host" >/dev/null 2>&1; }

if resolves; then
  exit 0
fi
echo "::warning::Runner DNS cannot resolve $probe_host; trying public resolvers for this job."

if ! command -v resolvectl >/dev/null 2>&1; then
  echo 'resolvectl is unavailable; leaving DNS unchanged.' >&2
  exit 0
fi

# First link that has DNS servers, for example "Link 2 (eth0): 168.63.129.16".
link_line="$(resolvectl dns 2>/dev/null | awk -F': ' '/^Link / && $2 != "" {print; exit}')"
if [[ ! "$link_line" =~ ^Link\ [0-9]+\ \(([A-Za-z0-9._-]+)\):\ (.+)$ ]]; then
  echo 'No DNS-bearing link found; leaving DNS unchanged.' >&2
  exit 0
fi
link="${BASH_REMATCH[1]}"
read -r -a current <<<"${BASH_REMATCH[2]}"

sudo -n resolvectl dns "$link" 1.1.1.1 9.9.9.9 "${current[@]}" || exit 0
sudo -n resolvectl flush-caches || true

if resolves; then
  echo "Public resolvers answer for $probe_host on $link."
else
  echo "Public resolvers did not answer for $probe_host either." >&2
fi
