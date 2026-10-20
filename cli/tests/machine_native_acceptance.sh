#!/usr/bin/env bash
# Destructive synthetic fixtures ONLY in an expendable native Linux VM/runner.
# Build the CLI and machine_context_spike with --locked before invoking this.
set -euo pipefail
[[ ${1:-} == --disposable && $# == 1 && $(uname -s) == Linux && $EUID == 0 ]] || {
  echo 'Run as root with --disposable in an expendable native Linux guest' >&2
  exit 2
}
: "${NYXID_TEST_CLI:?absolute path to the built CLI}"
: "${NYXID_TEST_CUA_DRIVER:?absolute path to the pinned cua driver}"
: "${NYXID_TEST_CONTEXT_SPIKE:?absolute path to the locked production-module fixture}"
for binary in "$NYXID_TEST_CLI" "$NYXID_TEST_CUA_DRIVER" "$NYXID_TEST_CONTEXT_SPIKE"; do
  [[ $binary == /* && -x $binary ]] || { echo 'Acceptance binaries must be absolute and executable' >&2; exit 2; }
done
for command in node python3 unshare mount certutil openssl cc c++ make cargo rustc npm; do
  command -v "$command" >/dev/null
done
test_directory=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
export NYXID_TEST_NATIVE_PROFILE="w94-$$"
node "$test_directory/machine_native_e2e.mjs" --disposable
python3 "$test_directory/machine_probe_acceptance.py" --disposable \
  --binary "$NYXID_TEST_CLI" \
  --config "/var/lib/nyxid-machine/$NYXID_TEST_NATIVE_PROFILE/node" \
  --samples "${NYXID_TEST_PROBE_SAMPLES:-100}"

# The existing adversarial suite requires a fresh root. Hide any earlier D
# fixture only in this private mount namespace; never erase or reuse its data.
mkdir -p /var/lib/nyxid-machine/m13-profile
fixture_directory=$(mktemp -d /var/lib/nyxid-machine/.w94-adversarial-XXXXXX)
chmod 0755 "$fixture_directory"
# Fixed-name /tmp and /dev/shm sentinels also need a fresh fixture. Keep the
# launcher/script outside those mounts, then replace only this harness's views.
cp "$test_directory/machine_context_spike/check.py" "$fixture_directory/check.py"
cp "$NYXID_TEST_CONTEXT_SPIKE" "$fixture_directory/launcher"
echo "Adversarial fixture directory=$fixture_directory"
unshare --mount --propagation private /bin/sh -eu -c '
  mount --bind "$1" /var/lib/nyxid-machine/m13-profile
  mount -t tmpfs -o mode=1777,nosuid,nodev tmpfs /tmp
  mount -t tmpfs -o mode=1777,nosuid,nodev tmpfs /dev/shm
  exec python3 "$1/check.py" --disposable --native-profile --launcher "$1/launcher"
' acceptance "$fixture_directory"
