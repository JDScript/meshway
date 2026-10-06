#!/bin/sh
set -eu
test "${MESHWAY_NODE_LAB_OWNED:-}" = 1
test -f /.dockerenv
test "$(id -u)" = 0
# exec keeps the service owner at PID 1; docker exec joins this mount namespace.
export MESHWAY_LAB_PARENT_MNT="$(readlink /proc/self/ns/mnt)"
export MESHWAY_LAB_PARENT_NET="$(readlink /proc/self/ns/net)"
exec unshare --mount --propagation private -- /usr/bin/python3 /usr/local/lib/meshway-lab/runtime.py "$@"
