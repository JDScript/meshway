#!/bin/sh
# Change only the exit fixture's own forwarding sysctls, never Docker's host
# namespace: `0|1` (IPv4 ip_forward), `ipv6 0|1` (all.forwarding),
# `accept-ra <interface> 0|1|2`.
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
case "${1:-}" in
    0|1) path=/proc/sys/net/ipv4/ip_forward; value=$1 ;;
    ipv6) path=/proc/sys/net/ipv6/conf/all/forwarding; value=${2:-} ;;
    accept-ra)
        case "${2:-}" in ''|*/*|.*) exit 2 ;; esac
        path=/proc/sys/net/ipv6/conf/$2/accept_ra; value=${3:-} ;;
    *) exit 2 ;;
esac
case "$value" in 0|1|2) ;; *) exit 2 ;; esac
test -e "$path"
mount --make-rprivate /
mount -o remount,bind,rw /proc/sys
printf '%s\n' "$value" > "$path"
test "$(cat "$path")" = "$value"
cat "$path"
