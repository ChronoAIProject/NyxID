#!/bin/sh
# The production verifier has no bypass. This runs a cfg(test)-only Docker
# adapter for unpublished local image bytes; live Sigstore/TUF tests are separate.
set -eu
name="nyxid-update-e2e-$(date +%s)-$$"
driver="$name-driver"
companion_image="nyxid-self-update-e2e:$$"
machine_image="${MACHINE_IMAGE:-nyxid-node-machine:machine-fixes}"
target_version=$(docker run --rm --entrypoint nyxid "$machine_image" --version)
target_version=${target_version#nyxid }
local_image="ghcr.io/chronoaiproject/nyxid/nyxid-node-machine:$target_version-e2e-$$"
cleanup() {
    test_status=$?
    if [ "$test_status" -ne 0 ]; then
        # The machine emits metadata-only diagnostics; capture them before the
        # failed fixture is removed so restart/readiness failures are actionable.
        docker logs --tail 80 "$name" >&2 || true
    fi
    for companion in $(docker ps -aq --filter "label=dev.nyxid.machine.updater=$name"); do
        if [ "$test_status" -ne 0 ]; then docker logs --tail 40 "$companion" >&2 || true; fi
        docker rm -f -v "$companion" >/dev/null 2>&1 || true
    done
    docker rm -f -v "$driver" "$name" >/dev/null 2>&1 || true
    for old in $(docker ps -a --filter "name=$name-nyxid-rollback-" --format '{{.ID}}'); do docker rm -f -v "$old" >/dev/null 2>&1 || true; done
    docker volume rm "$name-identity" "$name-workspace" "$name-browser-trust" "$name-nyxid-update" >/dev/null 2>&1 || true
    docker image rm "$companion_image-old" "$companion_image-new" "$companion_image-bad" >/dev/null 2>&1 || true
    docker image rm "$local_image" >/dev/null 2>&1 || true
}
trap cleanup EXIT INT TERM
docker image inspect ghcr.io/chronoaiproject/nyxid/nyxid-node-machine:0.40.0 >/dev/null 2>&1 || docker pull ghcr.io/chronoaiproject/nyxid/nyxid-node-machine:0.40.0
docker build --build-arg "MACHINE_IMAGE=$machine_image" -f cli/tests/Dockerfile.updater-browser -t "$local_image" .
updater_image="${UPDATER_E2E_IMAGE:-nyxid-machine-updater-e2e:local}"
if [ -z "${UPDATER_E2E_IMAGE:-}" ]; then
    docker build --target e2e -f cli/Dockerfile.machine-updater -t "$updater_image" .
fi
for variant in old new bad; do
    printf 'FROM %s\nLABEL dev.nyxid.test.companion=%s\n' "$updater_image" "$variant" | docker build -t "$companion_image-$variant" -
done
set --
if [ -n "${MACHINE_TEST_SECCOMP:-}" ]; then
    set -- --security-opt "seccomp=$MACHINE_TEST_SECCOMP" \
        --mount "type=bind,src=$MACHINE_TEST_SECCOMP,dst=/test/seccomp.json,readonly"
fi
docker run --rm --name "$driver" --mount type=bind,src=/var/run/docker.sock,dst=/var/run/docker.sock \
    --mount "type=volume,src=$name-nyxid-update,dst=/var/lib/nyxid-machine-update" \
    -e "NYXID_TEST_MACHINE=$name" -e "NYXID_TEST_DRIVER=$driver" -e "NYXID_TEST_IMAGE=$local_image" \
    -e "NYXID_TEST_TARGET_VERSION=$target_version" -e "NYXID_TEST_COMPANION_IMAGE=$companion_image" \
    "$@" "$updater_image"
