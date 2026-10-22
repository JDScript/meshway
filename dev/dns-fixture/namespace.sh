#!/bin/sh
# Only invoked by verify_runtime.py in its exact owned disposable actor container.
set -eu
if [ "${MESHWAY_DNS_FIXTURE_OWNED:-}" != 1 ] || [ ! -f /.dockerenv ]; then
    echo 'DNS fixture requires its owned Docker container' >&2
    exit 1
fi
if [ "${1:-}" != child ]; then
    export MESHWAY_DNS_PARENT_MNT="$(readlink /proc/self/ns/mnt)"
    export MESHWAY_DNS_PARENT_NET="$(readlink /proc/self/ns/net)"
    exec unshare --mount --propagation private -- /bin/sh /dns-fixture/namespace.sh child "$@"
fi
shift
test "$(readlink /proc/self/ns/mnt)" != "$MESHWAY_DNS_PARENT_MNT"
test "$(readlink /proc/self/ns/net)" = "$MESHWAY_DNS_PARENT_NET"
mount --make-rprivate /
{
    printf 'parentMount=%s\nchildMount=%s\nnetwork=%s\n' "$MESHWAY_DNS_PARENT_MNT" "$(readlink /proc/self/ns/mnt)" "$(readlink /proc/self/ns/net)"
    grep '^Cap' /proc/self/status
    cat /proc/self/mountinfo
} > /evidence/dns-namespace-before.txt
# Copy the image's /etc (including its own generated Docker metadata), never
# host /etc. The new mount hides every Docker network-file bind. No bind source
# is opened for writing, and no host propagation is possible after rprivate.
mkdir -m 700 /state/image-etc
cp -a /etc/. /state/image-etc/
umount /etc/resolv.conf
mount -t tmpfs -o mode=755,nosuid,nodev tmpfs /etc
cp -a /state/image-etc/. /etc/
rm -f /etc/resolv.conf
printf 'nameserver %s\noptions timeout:1 attempts:1\n' "$MESHWAY_DNS_UPSTREAM" > /etc/resolv.conf
chmod 644 /etc/resolv.conf
test "$(stat -c %h /etc/resolv.conf)" = 1
test "$(stat -c %u /etc/resolv.conf)" = 0
test "$(findmnt -n -o ID -T /etc/resolv.conf)" = "$(findmnt -n -o ID -T /etc)"
mkdir -p /run/dbus /run/systemd
bus=
resolved=
actor=
cleanup() {
    trap - EXIT INT TERM
    for pid in "$actor" "$resolved" "$bus"; do
        if [ -n "$pid" ]; then kill -TERM "$pid" 2>/dev/null || true; fi
    done
    for pid in "$actor" "$resolved" "$bus"; do
        if [ -n "$pid" ]; then
            attempts=0
            while kill -0 "$pid" 2>/dev/null && [ "$attempts" -lt 100 ]; do
                attempts=$((attempts + 1)); sleep .1
            done
            kill -KILL "$pid" 2>/dev/null || true
            wait "$pid" 2>/dev/null || true
        fi
    done
}
trap cleanup EXIT
trap 'exit 143' TERM
trap 'exit 130' INT
if [ "$MESHWAY_DNS_BACKEND" = resolved ]; then
    rm -f /etc/machine-id
    tr -d '-' < /proc/sys/kernel/random/uuid > /etc/machine-id
    chmod 444 /etc/machine-id
    cat > /etc/systemd/resolved.conf <<EOF
[Resolve]
DNS=$MESHWAY_DNS_UPSTREAM
FallbackDNS=
DNSSEC=no
DNSOverTLS=no
LLMNR=no
MulticastDNS=no
DNSStubListener=yes
Cache=no
EOF
    # The managed stub file publishes link search domains to libc. The static
    # /usr/lib file only lists the stub address and cannot provide that contract.
    rm /etc/resolv.conf
    ln -s /run/systemd/resolve/stub-resolv.conf /etc/resolv.conf
    # Installed Debian bus policies admit root and systemd-resolve only as
    # configured by the package; this is an actor-private system bus.
    dbus-daemon --system --nofork --nopidfile > /evidence/dbus.log 2>&1 &
    bus=$!
    attempts=0
    until [ -S /run/dbus/system_bus_socket ]; do
        kill -0 "$bus"
        attempts=$((attempts + 1)); test "$attempts" -lt 100
        sleep .1
    done
    /usr/lib/systemd/systemd-resolved > /evidence/resolved.log 2>&1 &
    resolved=$!
    attempts=0
    until resolvectl status > /evidence/resolved-before.txt 2>/dev/null; do
        kill -0 "$resolved"
        attempts=$((attempts + 1)); test "$attempts" -lt 100
        sleep .1
    done
    { resolvectl dns; resolvectl domain; } > /evidence/resolved-settings-before.txt
fi
cat /proc/self/mountinfo > /evidence/dns-namespace-after.txt
cp /usr/share/meshway-dns-packages.tsv /evidence/os-packages.tsv
# The actor's SIGUSR1 asks for one resolved restart: a new process and D-Bus
# owner with the same configuration, as `systemctl restart` does. At most
# MESHWAY_DNS_RESTARTS (default 1) per run; restart n also writes
# resolved-restarted-<n>.txt.
restarts=0
restart_resolved() {
    if [ "$MESHWAY_DNS_BACKEND" != resolved ] || [ "$restarts" -ge "${MESHWAY_DNS_RESTARTS:-1}" ]; then
        return
    fi
    restarts=$((restarts + 1))
    old=$resolved
    kill -TERM "$old"
    wait "$old" 2>/dev/null || true
    /usr/lib/systemd/systemd-resolved >> /evidence/resolved.log 2>&1 &
    resolved=$!
    attempts=0
    until resolvectl status > /evidence/resolved-restarted-status.txt 2>/dev/null; do
        # No marker: the actor reports the failed restart.
        kill -0 "$resolved" 2>/dev/null || return
        attempts=$((attempts + 1)); [ "$attempts" -lt 100 ] || return
        sleep .1
    done
    if [ "$restarts" -eq 1 ]; then
        printf 'old=%s new=%s\n' "$old" "$resolved" > /evidence/resolved-restarted.txt
    fi
    printf 'old=%s new=%s\n' "$old" "$resolved" > "/evidence/resolved-restarted-$restarts.txt"
}
trap restart_resolved USR1
"$@" &
actor=$!
set +e
# A trapped signal interrupts wait; keep waiting for the actor itself.
while :; do
    wait "$actor"
    status=$?
    kill -0 "$actor" 2>/dev/null || break
done
set -e
actor=
if [ "$MESHWAY_DNS_BACKEND" = resolved ]; then
    resolvectl status > /evidence/resolved-after.txt || status=1
    { resolvectl dns; resolvectl domain; } > /evidence/resolved-settings-after.txt || status=1
    cmp /evidence/resolved-settings-before.txt /evidence/resolved-settings-after.txt || status=1
fi
exit "$status"
