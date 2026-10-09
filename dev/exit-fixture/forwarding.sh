#!/bin/sh
# Change only the exit fixture's IPv4 sysctl, never Docker's host namespace.
set -eu
test "${MESHWAY_EXIT_FIXTURE_OWNED:-}" = 1
test -f /.dockerenv
if [ "${1:-}" != child ]; then
    export MESHWAY_EXIT_PARENT_MNT="$(readlink /proc/self/ns/mnt)"
    export MESHWAY_EXIT_PARENT_NET="$(readlink /proc/self/ns/net)"
    exec unshare --mount --propagation private -- /bin/sh /dns-fixture/forwarding.sh child "$@"
fi
shift
test "$(readlink /proc/self/ns/mnt)" != "$MESHWAY_EXIT_PARENT_MNT"
test "$(readlink /proc/self/ns/net)" = "$MESHWAY_EXIT_PARENT_NET"
case "${1:-}" in 0|1) ;; *) exit 2 ;; esac
mount --make-rprivate /
mount -o remount,bind,rw /proc/sys
printf '%s\n' "$1" > /proc/sys/net/ipv4/ip_forward
test "$(cat /proc/sys/net/ipv4/ip_forward)" = "$1"
cat /proc/sys/net/ipv4/ip_forward
