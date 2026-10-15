#!/bin/sh
# One NAT router of the roaming rows in a newly labelled fixture container:
# port-preserving SNAT for its one actor and port-restricted filtering (only
# replies of flows the actor started enter). Mode `forward` also forwards UDP
# 35000 on its WAN address to the actor (a published listener); mode `nat`
# forwards nothing. No host namespace, TUN, published port or host service is
# touched.
set -eu
umask 077
test "${MESHWAY_ROAM_FIXTURE_OWNED:-}" = 1
test "$#" = 4
lan=$1
wan=$2
actor=$3
mode=$4
case "$mode" in forward|nat) ;; *) exit 2 ;; esac
iface() {
    ip -o -4 address show | awk -v address="$1" 'split($4,a,"/") && a[1] == address {print $2}'
}
lan_if=$(iface "$lan")
wan_if=$(iface "$wan")
test -n "$lan_if" && test -n "$wan_if" && test "$lan_if" != "$wan_if"
test "$(sysctl -n net.ipv4.ip_forward)" = 1
iptables -P FORWARD DROP
iptables -A FORWARD -i "$lan_if" -o "$wan_if" -s "$actor/32" -j ACCEPT
iptables -A FORWARD -i "$wan_if" -o "$lan_if" -d "$actor/32" -m conntrack --ctstate ESTABLISHED,RELATED,DNAT -j ACCEPT
iptables -A FORWARD -j DROP
if test "$mode" = forward; then
    iptables -t nat -A PREROUTING -i "$wan_if" -d "$wan/32" -p udp --dport 35000 -j DNAT --to-destination "$actor:35000"
else
    # No router-local UDP service: an unsolicited WAN datagram is dropped,
    # never answered as if a listener were here.
    iptables -A INPUT -i "$wan_if" -p udp -m conntrack --ctstate NEW -j DROP
fi
iptables -t nat -A POSTROUTING -s "$actor/32" -o "$wan_if" -j SNAT --to-source "$wan"
iptables-save -c > /evidence/firewall-ready.txt
finish() {
    trap - EXIT INT TERM
    iptables-save -c > /evidence/firewall-final.txt || :
    conntrack -L -p udp -o extended > /evidence/conntrack-final.txt 2> /evidence/conntrack-final.stderr || :
}
trap finish EXIT
trap 'exit 1' INT TERM
printf '{"kind":"ready"}\n' > /evidence/.ready.tmp
mv /evidence/.ready.tmp /evidence/ready.json
count=0
while test ! -f /signals/roam-stop; do
    count=$((count + 1))
    test "$count" -lt 36000
    sleep 0.5
done
