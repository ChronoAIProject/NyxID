#!/bin/sh
# Fixed help/version dispatch before machine startup side effects. Ordinary
# startup execs the original entrypoint with identical arguments/environment.
set -eu
[ "$#" -ge 1 ] || exit 2
entrypoint=$1
shift
case "$entrypoint" in
    /usr/local/bin/nyxid-machine-entrypoint|/usr/local/bin/nyxid-machine-node-only) ;;
    *) echo 'Unrecognized machine image entrypoint.' >&2; exit 2 ;;
esac
if [ "$#" -eq 1 ]; then
    case "$1" in
        --help|--version) exec nyxid "$1" ;;
    esac
fi
exec "$entrypoint" "$@"
