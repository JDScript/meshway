#!/bin/sh
# Only entered in a newly labelled two-interface fixture container. No host NS,
# TUN, host port publication or host network service mutation. Optional DNAT is
# TCP-only and enabled later by the explicit run-private transition signal.
set -eu
umask 077
test "${MESHWAY_TRAVERSAL_OWNED:-}" = 1
test "$#" = 4
role=$1
private=$2
wan=$3
actor=$4
case "$role" in router-a|router-r) ;; *) exit 2 ;; esac
iface() {
    ip -o -4 address show | awk -v address="$1" 'split($4,a,"/") && a[1] == address {print $2}'
}
lan_if=$(iface "$private")
wan_if=$(iface "$wan")
test -n "$lan_if" && test -n "$wan_if" && test "$lan_if" != "$wan_if"
test "$(printf '%s\n' "$lan_if" | wc -l)" -eq 1
test "$(printf '%s\n' "$wan_if" | wc -l)" -eq 1
ip -j address show > /evidence/addresses.json
ip -j -4 route show table all > /evidence/routes-before.json
ip -j -6 route show table all > /evidence/routes6-before.json
iptables-save -c > /evidence/firewall-before.txt
# Docker initializes these namespaced values before starting this process;
# /proc/sys remains read-only under the ordinary container confinement.
test "$(sysctl -n net.ipv4.ip_forward)" = 1
sysctl net.ipv4.ip_forward > /evidence/forwarding.txt
# IPv6 remains enabled on Nodes for inner TUN and DAD. This IPv4-only router
# never forwards IPv6 between the isolated documentation-prefix bridges.
test "$(sysctl -n net.ipv6.conf.all.forwarding)" = 0
sysctl net.ipv6.conf.all.forwarding >> /evidence/forwarding.txt
iptables -P FORWARD DROP
iptables -A FORWARD -i "$lan_if" -o "$wan_if" -s "$actor/32" -j ACCEPT
iptables -A FORWARD -i "$wan_if" -o "$lan_if" -d "$actor/32" -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT
iptables -A FORWARD -j DROP
iptables -t nat -A POSTROUTING -s "$actor/32" -o "$wan_if" -j SNAT --to-source "$wan"
# No router-local UDP listener: unsolicited WAN packets must not be mistaken
# for a listening peer or an implicit UDP forward.
iptables -A INPUT -i "$wan_if" -p udp -m conntrack --ctstate NEW -j DROP
if test "${MESHWAY_TRAVERSAL_FALLBACK:-0}" = 1; then
    test -n "${MESHWAY_TRAVERSAL_PEER_WAN:-}"
    if test "$role" = router-r; then
        iptables -A INPUT -i "$wan_if" -p tcp --dport 35001 -j REJECT --reject-with tcp-reset
    fi
fi
iptables-save -c > /evidence/firewall-ready.txt
ip -j -4 route show table all > /evidence/routes-ready.json
wan_pid=
lan_pid=
finish() {
    trap - EXIT INT TERM
    for pid in "$wan_pid" "$lan_pid"; do
        if test -n "$pid"; then kill -INT "$pid" 2>/dev/null || :; fi
    done
    result=0
    for pid in "$wan_pid" "$lan_pid"; do
        if test -n "$pid"; then wait "$pid" || result=1; fi
    done
    iptables-save -c > /evidence/firewall-final.txt || result=1
    conntrack -L -p udp -s "$actor" -o extended > /evidence/conntrack-final.txt 2>/evidence/conntrack-final.stderr || result=1
    if test "${MESHWAY_TRAVERSAL_FALLBACK:-0}" = 1; then
        conntrack -L -p tcp -o extended > /evidence/conntrack-tcp-final.txt 2>/evidence/conntrack-tcp-final.stderr || result=1
    fi
    test "$result" = 0 || exit 1
}
trap finish EXIT
trap 'exit 1' INT TERM
# Fixed packet count and snaplen bound each file to <7 MiB. Capture on each
# physical side records real SNAT, not the same packet twice on an any device.
filter='udp and (port 3478 or port 35000)'
if test "${MESHWAY_TRAVERSAL_FALLBACK:-0}" = 1; then
    filter='(udp and (port 3478 or port 35000)) or (tcp and port 35001)'
fi
tcpdump -i "$wan_if" -nn -U -s 128 -c 40000 -Z root -w /evidence/wan.pcap "$filter" 2>/evidence/wan-capture.stderr &
wan_pid=$!
tcpdump -i "$lan_if" -nn -U -s 128 -c 40000 -Z root -w /evidence/private.pcap "$filter" 2>/evidence/private-capture.stderr &
lan_pid=$!
# Wait until both libpcap headers have actually been created before actors run.
count=0
while test ! -s /evidence/wan.pcap || test ! -s /evidence/private.pcap; do
    kill -0 "$wan_pid" && kill -0 "$lan_pid"
    count=$((count + 1))
    test "$count" -lt 100
    sleep 0.1
done
printf '{"kind":"ready"}\n' > "/signals/.$role-ready.tmp"
mv "/signals/.$role-ready.tmp" "/signals/$role-ready.json"
count=0
while test ! -f /signals/routers-stop.json; do
    if test "${MESHWAY_TRAVERSAL_FALLBACK:-0}" = 1 && test -f /signals/enable-fallback.json && test ! -f /evidence/fallback.json; then
        if test "$role" = router-r; then
            iptables-save -c > /evidence/firewall-before-fallback.txt
            enable_started=$(date +%s%3N)
            iptables -t nat -A PREROUTING -i "$wan_if" -d "$wan/32" -p tcp --dport 35001 -j DNAT --to-destination "$actor:35001"
            iptables -I FORWARD 1 -i "$wan_if" -o "$lan_if" -d "$actor/32" -p tcp --dport 35001 -m conntrack --ctstate NEW,ESTABLISHED -j ACCEPT
            printf '{"enableStartedAtMs":%s,"enabledAtMs":%s}\n' "$enable_started" "$(date +%s%3N)" > /evidence/fallback.json
            iptables-save -c > /evidence/firewall-fallback.txt
            printf '{"kind":"ready"}\n' > /signals/.fallback-router-r.tmp
            mv /signals/.fallback-router-r.tmp /signals/fallback-router-r.json
        elif test -f /signals/fallback-router-r.json; then
            iptables-save -c > /evidence/firewall-before-fallback.txt
            # Match the exact peer IP pair, independent of any later mapping
            # port. STUN/HTTPS destinations and R-B UDP remain untouched.
            iptables -I FORWARD 1 -i "$lan_if" -o "$wan_if" -s "$actor/32" -d "$MESHWAY_TRAVERSAL_PEER_WAN/32" -p udp -j DROP
            iptables -I FORWARD 1 -i "$wan_if" -o "$lan_if" -s "$MESHWAY_TRAVERSAL_PEER_WAN/32" -d "$actor/32" -p udp -j DROP
            printf '{"blockedAtMs":%s}\n' "$(date +%s%3N)" > /evidence/fallback.json
            cp /evidence/fallback.json /signals/.fallback-blocked-time.tmp
            mv /signals/.fallback-blocked-time.tmp /signals/fallback-blocked-time.json
            iptables-save -c > /evidence/firewall-fallback.txt
            printf '{"kind":"ready"}\n' > /signals/.fallback-router-a.tmp
            mv /signals/.fallback-router-a.tmp /signals/fallback-router-a.json
        fi
    fi
    # nat/relay: drop every packet between this actor and B (direct A–B).
    if test -n "${MESHWAY_TRAVERSAL_RELAY_PEER:-}" && test -f /signals/relay-block.json && test ! -f /evidence/relay.json; then
        iptables-save -c > /evidence/firewall-before-relay.txt
        iptables -I FORWARD 1 -i "$lan_if" -o "$wan_if" -s "$actor/32" -d "$MESHWAY_TRAVERSAL_RELAY_PEER/32" -j DROP
        iptables -I FORWARD 1 -i "$wan_if" -o "$lan_if" -s "$MESHWAY_TRAVERSAL_RELAY_PEER/32" -d "$actor/32" -j DROP
        printf '{"blockedAtMs":%s}\n' "$(date +%s%3N)" > /evidence/relay.json
        iptables-save -c > /evidence/firewall-relay.txt
        printf '{"kind":"ready"}\n' > "/signals/.relay-blocked-$role.tmp"
        mv "/signals/.relay-blocked-$role.tmp" "/signals/relay-blocked-$role.json"
    fi
    # nat/reconnect: at the first break A's router maps every new UDP flow to
    # a random port (endpoint-dependent mapping) and R's admits inbound UDP
    # (R behaves as a public host). Each break drops A↔R UDP until healed;
    # healing also forgets every A mapping, as a NAT rebinding does.
    if test "${MESHWAY_TRAVERSAL_RECONNECT:-0}" = 1; then
        for stage in nat server; do
            if test -f "/signals/reconnect-break-$stage.json" && test ! -f "/evidence/reconnect-break-$stage.json"; then
                if test ! -f /evidence/reconnect-mode.json; then
                    if test "$role" = router-a; then
                        iptables -t nat -I POSTROUTING 1 -s "$actor/32" -o "$wan_if" -p udp -j SNAT --to-source "$wan" --random-fully
                    else
                        iptables -t nat -A PREROUTING -i "$wan_if" -d "$wan/32" -p udp -j DNAT --to-destination "$actor"
                        iptables -I FORWARD 1 -i "$wan_if" -o "$lan_if" -d "$actor/32" -p udp -j ACCEPT
                    fi
                    printf '{"atMs":%s}\n' "$(date +%s%3N)" > /evidence/reconnect-mode.json
                    iptables-save -c > /evidence/firewall-reconnect-mode.txt
                fi
                if test "$role" = router-a; then
                    test -n "${MESHWAY_TRAVERSAL_RECONNECT_PEER:-}"
                    iptables -I FORWARD 1 -i "$lan_if" -o "$wan_if" -s "$actor/32" -d "$MESHWAY_TRAVERSAL_RECONNECT_PEER/32" -p udp -j DROP
                    iptables -I FORWARD 1 -i "$wan_if" -o "$lan_if" -s "$MESHWAY_TRAVERSAL_RECONNECT_PEER/32" -d "$actor/32" -p udp -j DROP
                fi
                printf '{"blockedAtMs":%s}\n' "$(date +%s%3N)" > "/evidence/reconnect-break-$stage.json"
                printf '{"kind":"ready"}\n' > "/signals/.reconnect-blocked-$stage-$role.tmp"
                mv "/signals/.reconnect-blocked-$stage-$role.tmp" "/signals/reconnect-blocked-$stage-$role.json"
            fi
            if test -f "/signals/reconnect-heal-$stage.json" && test -f "/evidence/reconnect-break-$stage.json" \
                    && test ! -f "/evidence/reconnect-heal-$stage.json"; then
                if test "$role" = router-a; then
                    iptables-save -c > "/evidence/firewall-reconnect-$stage.txt"
                    iptables -D FORWARD -i "$lan_if" -o "$wan_if" -s "$actor/32" -d "$MESHWAY_TRAVERSAL_RECONNECT_PEER/32" -p udp -j DROP
                    iptables -D FORWARD -i "$wan_if" -o "$lan_if" -s "$MESHWAY_TRAVERSAL_RECONNECT_PEER/32" -d "$actor/32" -p udp -j DROP
                    conntrack -D -p udp -s "$actor" > /dev/null 2>&1 || :
                fi
                printf '{"healedAtMs":%s}\n' "$(date +%s%3N)" > "/evidence/reconnect-heal-$stage.json"
                printf '{"kind":"ready"}\n' > "/signals/.reconnect-healed-$stage-$role.tmp"
                mv "/signals/.reconnect-healed-$stage-$role.tmp" "/signals/reconnect-healed-$stage-$role.json"
            fi
        done
    fi
    if test -f /signals/traversal-first.json && test ! -f /evidence/conntrack-first.txt; then
        conntrack -L -p udp -s "$actor" -o extended > /evidence/conntrack-first.txt 2>/evidence/conntrack-first.stderr
        iptables-save -c > /evidence/firewall-first.txt
    fi
    if test -f /signals/traffic.json && test ! -f /evidence/conntrack-traffic.txt; then
        conntrack -L -p udp -s "$actor" -o extended > /evidence/conntrack-traffic.txt 2>/evidence/conntrack-traffic.stderr
        iptables-save -c > /evidence/firewall-traffic.txt
        if test "${MESHWAY_TRAVERSAL_FALLBACK:-0}" = 1; then
            conntrack -L -p tcp -o extended > /evidence/conntrack-tcp-traffic.txt 2>/evidence/conntrack-tcp-traffic.stderr
        fi
        # The controller revokes access only after this live snapshot exists.
        printf '{"kind":"ready"}\n' > "/signals/.traffic-$role.tmp"
        mv "/signals/.traffic-$role.tmp" "/signals/traffic-$role.json"
    fi
    count=$((count + 1))
    test "$count" -lt 1800
    sleep 1
done
