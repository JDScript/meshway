# Isolated IPv4 exit fixture (preparation)

`exit/off` is explicit-only and **expected to fail with `unsupported`** until
X2–X5 are connected. It starts six infrastructure actors, not a working exit
product. Controller/A/R/E currently rendezvous and record pending stages; no
Server configuration, daemon installation, permit, flow table or NAT is faked.

```text
             Controller (forwarding=0)
              |ca      |cr       |ce      (three separate control bridges)
              A --ar-- R --re-- E
                               |external -- IPv4 service :8080 (TEST-NET-2)
                               |lan ------ LAN service :8080 (RFC1918)
```

All seven bridges are run-labelled, internal Docker networks with no published
ports or Internet route. `ar` and `re` are separate physical adjacencies; A and E
share neither data nor control bridge. Only E has an interface on each service
network. Controller/R kernel forwarding is disabled. The future Mesh path is
A→R→E and the reply must be E→R→A, not newly routed through Controller.

## Addresses and classification

Docker IPAM allocates six RFC1918 control/data/LAN subnets, checked before use.
The external link requests a random /29 in `198.51.100.0/24` (TEST-NET-2);
conflicting allocations fail without touching existing networks. Each link's
first role in the diagram gets subnet +2, the second +3; Docker reserves its
gateway separately. `report.json.addresses` records exact per-run assignments.

RFC1918 echo cannot prove Internet authorization: it is an exit-LAN target and
requires explicit LAN permission. TEST-NET is **also excluded by the proposed
production Internet classifier**. Here it provides an isolated non-RFC1918
external simulation address, not a production classification exception. A
future IP/CIDR test can use an explicit target permit if supported; an Internet
grant success still needs a fixture-owned, globally classified address isolated
from real routing and agreed with X1. No reserved prefix is silently declared
public and no product allow-list is changed to make this row pass.

## Image and invocation

The sole image lock is `dev/node-lab/assets.lock.json`: Ubuntu digest, snapshot
Release/index SHA-256, dependency closure and individual deb SHA-256. It includes
`nftables`, `conntrack` (conntrack-tools), `tcpdump`, `curl` and `iproute2`.
`dev/node-lab/build.py` checks required tool packages/paths and the offline image
build checks installed versions. There is no second exit image lock or Rust
build in Docker; native host-built manifest-checked binaries are mounted read-only.

```sh
mise run node:lab image-build
mise run test:runtime:linux --build-only
mise run test:runtime:linux --artifacts /absolute/private/bin --transport native-udp --dns-backend off --traversal exit
# Or one build, explicit row (never part of the default matrix):
mise run validate:matrix --rows exit/off --short
```

The matrix selects the same provenance-checked tools image without changing
the persistent lab selection. Changed image inputs require explicit image-build
before using an old selected image. No lab deployment is part of preparation.

Each actor has a read-only root, private state tmpfs, no-new-privileges, dropped
capabilities, 2 CPU/256 MiB/64 PID limits and the existing runtime ownership
label. A/R/E have NET_ADMIN and `/dev/net/tun`; R/E also have NET_RAW and
SETUID/SETGID for tcpdump's explicit root capture identity in the private bind.
Only E declares `--sysctl net.ipv4.ip_forward=1`, in its own namespace. All other
actors declare forwarding=0. No host network, privileged, host sysctl change,
Docker socket mount or host VM access is used.

## Stage contract

The Rust `exit::Stage` enumeration follows exit-design.md steps 1–8:

| Stage | Required product evidence | Current status |
| --- | --- | --- |
| 1 Install | Admin configuration, role, real saved/applied and resources | pending-x2-x5 |
| 2 IPv4 NAT and reply | TCP/MSS, UDP/MTU, echo, NAT tuple, encrypted R, original reply, 125s renewal | pending-x2-x5 |
| 3 Denials | Source/port/LAN/public classifier, path, reply, hairpin, IPv6 | pending-x2-x5 |
| 4 Withdrawal | Access/exit/member/path/profile, precise nft/ct retirement, fresh-only recovery | pending-x2-x5 |
| 5 Expiry/cancellation | HTTP loss through actual expiry, blocked output/ACK | pending-x2-x5 |
| 6 Host failures | forwarding/capability/nft/rp_filter/conflict/crash/journal | pending-x2-x5 |
| 7 Capture/bypass | /0 with protected control/peer and no application bypass, first-match/IPv6 sink | pending-x2-x5 |
| 8 Offline cleanup | Exact product resources removed, sentinels unchanged, all children joined | pending-x2-x5 |

Infrastructure cleanup is implemented independently of pending product cleanup.
Every actor stops on the supervisor marker, returns a nonzero unsupported result,
and is inspected before exact removal; the matrix independently checks leftovers.

## Evidence and E-block control

- `baseline.json` / `after-probes.json`: namespace IDs, interfaces, routes, RPDB,
  E forwarding, read-only `nft -j list ruleset` (including counters), and
  `conntrack -L -f ipv4 -d <external-or-LAN-target> -o extended`. No flush,
  deletion, masquerade or product nft installation is performed by the collector.
- R and E capture all non-loopback interfaces separately, covering both mesh
  adjacencies and each E service egress, with 4096-packet/256-byte snaplen bounds.
  Capture children are interrupted and joined before decoding `*-tuples.json`.
  Future NAT proof must correlate the egress source/port with the original and
  reply conntrack tuples and product rule counters; an empty dump stays empty.
- External and LAN Rust service actors keep bounded `access.json` peer-address
  logs. Current probes originate **locally from E**, proving only collector and
  topology health, never masquerade or A→R→E business success.
- `infrastructure-probes.json` checks E→service success, A/R/Controller→service
  failure, then adds an exact fixture-owned unreachable /32 route on E for the
  service, observes failure, deletes that exact route and observes success. This
  keeps the egress interface's capture socket alive and never replaces a route.
  Future business control must repeat the
  same window with A's live flow, correlate service logs and E's pcap, and verify
  no fallback on A or shortcut around R. It remains `pending-x2-x5` now.
- `exit-stages.json`, source/binary hashes, image provenance, Docker inspection,
  pcap SHA-256 and process results are retained. No IP-check equality, missing
  sample, local E probe, empty counter or collector completion becomes a pass.
