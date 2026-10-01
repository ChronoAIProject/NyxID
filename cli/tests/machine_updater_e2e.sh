#!/bin/sh
# The production verifier has no bypass. This runs a cfg(test)-only Docker
# adapter for unpublished local image bytes; live Sigstore/TUF tests are separate.
set -eu
name="nyxid-update-e2e-$(date +%s)-$$"
driver="$name-driver"
machine_image="${MACHINE_IMAGE:-nyxid-node-machine:machine-fixes}"
target_version=$(docker run --rm --entrypoint nyxid "$machine_image" --version)
target_version=${target_version#nyxid }
local_image="ghcr.io/chronoaiproject/nyxid/nyxid-node-machine:$target_version-e2e-$$"
cleanup() {
    docker rm -f -v "$driver" "$name" >/dev/null 2>&1 || true
    for old in $(docker ps -a --filter "name=$name-nyxid-rollback-" --format '{{.ID}}'); do docker rm -f -v "$old" >/dev/null 2>&1 || true; done
    docker volume rm "$name-identity" "$name-workspace" "$name-nyxid-update" >/dev/null 2>&1 || true
    docker image rm "$local_image" >/dev/null 2>&1 || true
}
trap cleanup EXIT INT TERM
docker image inspect ghcr.io/chronoaiproject/nyxid/nyxid-node-machine:0.40.0 >/dev/null 2>&1 || docker pull ghcr.io/chronoaiproject/nyxid/nyxid-node-machine:0.40.0
docker tag "$machine_image" "$local_image"
updater_image="${UPDATER_E2E_IMAGE:-nyxid-machine-updater-e2e:local}"
docker build --target e2e -f cli/Dockerfile.machine-updater -t "$updater_image" .
set --
if [ -n "${MACHINE_TEST_SECCOMP:-}" ]; then
    set -- --security-opt "seccomp=$MACHINE_TEST_SECCOMP" \
        --mount "type=bind,src=$MACHINE_TEST_SECCOMP,dst=/test/seccomp.json,readonly"
fi
docker run --rm --name "$driver" --mount type=bind,src=/var/run/docker.sock,dst=/var/run/docker.sock \
    --mount "type=volume,src=$name-nyxid-update,dst=/var/lib/nyxid-machine-update" \
    -e "NYXID_TEST_MACHINE=$name" -e "NYXID_TEST_DRIVER=$driver" -e "NYXID_TEST_IMAGE=$local_image" \
    -e "NYXID_TEST_TARGET_VERSION=$target_version" \
    "$@" "$updater_image"
