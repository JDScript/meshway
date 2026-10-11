# Isolated IPv4 exit acceptance

`exit/off` runs the real Server, three independent daemons/CLI instances and two
external application services. It is a default matrix row. A short run is
diagnostic evidence; a component pass is not a row pass. Current acceptance
status is defined in `docs/v2/node/validation.md`.

```text
              Controller (forwarding=0)
               |ca      |cr       |ce
               A --ar-- R --re-- E
               |split            |external -- isolated public-classified service + E's DNS authority
            Resolver             |lan ------ RFC1918 service
   (A's OS DNS upstream and useWithExit split resolver)
```

## Configuration and isolation

Eight labelled internal bridges have no published ports or Internet route.
A/E share neither a control nor a data link. Only E attaches to the service
networks. Six RFC1918 links use Docker IPAM; the external link uses a random
`11.77.x.0/29` entirely inside its internal bridge. This address is eligible for
Policy's Internet classification. The accepted permit must explicitly carry
`ExitDestinationScope::Internet`; TEST-NET/RFC1918 cannot satisfy that assertion.
Allocation conflicts fail without changing another network.

`controller/exit.rs` submits configuration through the generated management
client: only E has `exitEnabled`; only A is assigned the `exit` profile, whose
fallback is Exit to E via R. R/E keep `mesh`. Access is source A to
`autogroup:internet` and the exact allocated LAN CIDR, limited to TCP 8080, UDP
8080/8081 and ICMP. Demand-7 topology permits A–R and R–E with Native UDP/TCP
declarations. No separate permit issuer or NAT implementation exists here.

DNS: MagicDNS is on; one UDP resolver `split` (the Resolver's `split` address,
`useWithExit: true`) serves `split.test`, and A's `exit` profile has a direct
rule for exactly that /32. Every Node runs the existing private
D-Bus/resolved namespace wrapper. A's resolved global upstream is the Resolver,
which answers *any* name outside `split.test` with a marker address: a source
fallback would succeed and be logged. E's only upstream is the external
authority (`*.exit-dns.test`; `big.exit-dns.test` has 90 A records). R's
upstream is its unanswered control address. Both authorities log every
question with its peer.

Containers use the node-lab digest-locked OS image with manifest-checked,
read-only host-built executables. Each actor has a read-only root, private
state, 2 CPU/768 MiB/128 PID limits, no-new-privileges and dropped capabilities.
A/R/E receive TUN/NET_ADMIN plus the existing resolved namespace capabilities.
No host network, privileged container, Docker socket mount or host VM access
is used. All local mutations are in fixture-owned namespaces.

## Invocation

```sh
mise run validate:matrix --rows exit/off --short
git push planck <branch>
mise run validate:remote -- --ref <commit> --rows exit/off
```

The shared runtime build includes the production underlay library and netmap
test binaries. `components.py` runs the netmap exit-role owner (renewal,
unchanged serial, Access independence, withdrawal cleanup), its projection
capacity refusal (one flow refused, role kept), the nft transaction and the
live exit-prerequisite probe in separate namespaces.
Their results are labelled `scope=component`; the main actors still must
prove their own installation, NAT and traffic lifecycle.

## Assertion and evidence boundaries

| Phase | Assertions | Main evidence |
| --- | --- | --- |
| 1 Capability | Accepted acquire declares `feature:exit-ipv4` v1; capability instance equals live instance | `e/acquisition.json`, `e/installed-status.json` |
| 2 Prerequisite | `ip_forward=0` gives `ipForwardDisabled`, empty installed families and no role; restore reads back 1 | `ip-forward-disabled.json`, `forwarding-restored.json` |
| 3 Applied/DNS | Ready IPv4 role, exact saved/applied/lease netmap reference, real kernel resources; failed system DNS query with no plaintext query on A's physical interfaces | `installed.json`, `e/installed-status.json`, `negative-dns-query.json`, `a/*.pcap` |
| 4 Capture/transit | A default capture routes the external IP to its actual TUN; cached netmaps show A's IPv6 sink, R without and E with an exit role | `capture-before-admission.json`, `capture-route.json`, `original-path.json` |
| 5 Projection/NAT | Original and reverse conntrack tuple/owned mark match external service log; exact ACK/first-write/confirmation ordering is checked by the production driver test | `nat-proof.json`, `first-flow.json`, `components/driver.log` |
| 6 Renewal/failure | Persistent TCP/UDP and receive-only UDP; driver checks original cutoff, ConfirmFailed exact isolation and surviving flow | `a/traffic-*.json`, `a/receive-only-*.json`, `components/{driver,handoff}.log` |
| 7 Reply/path | Real TCP/UDP/echo, zero R TUN delivery and no payload in R pcap; A's egress partition sends both targets Exit→E via R and all three projections carry the same external Access, so only `autogroup:internet` covers the public target | `original-path.json`, `r/*.pcap`, `icmp-*.json`, `business-block-control.json` |
| 9 DNS | A's plan is `viaExit` on A→R→E and E serves exactly A; system `getaddrinfo` on A returns E's answers (A, AAAA, 90-record answer that only fits over the Mesh TCP route) and the external authority's only asker is E's external address; A's own MagicDNS name and an unknown one stay local; the split name reaches the Resolver from A's split address only; with E's OS upstream down (the authority stops answering; no route or link change on E) A's query fails and the Resolver never sees it; DNS fails on role withdrawal and lease expiry, works offline within the lease and recovers; `exit-dns` bytes appear only in E's external capture | `dns-exit.json`, `dns-lifecycle.json`, `external/dns-queries.json`, `resolver/dns-queries.json`, `*/*.pcap` |
| 8 Drain/lease | Role withdrawal and old socket refusal; Node HTTP closed (in-flight sync polls ended, new requests held): `lease.py` keeps a TCP echo and new UDP flows working offline until the first path lease expiry, then they end; E withdraws its role at its own expiry with exact nft/conntrack cleanup and A keeps its negative egress gate; reopened HTTP recovers a new lease and new flows without a second up; SIGKILL/same-netns journal recovery, explicit down and foreign sentinel preservation | `withdrawn.json`, `lease-window.json`, `lease-*.json`, `expired.json`, `recovered.json`, `*crash*.json`, `down.json`, `foreign-sentinel-*.json` |

The source uses a 30-second short window or 125-second full window. Public and
LAN TCP/UDP windows run concurrently, alongside independent receive-only UDP.
TCP sends 16 KiB across MSS boundaries; UDP includes 1338-byte IPv4 payload.
The service keeps bounded peer-address access logs. The supervisor compares
original and NAT tuples and performs E-only unreachable-route block/restore
controls from A. Kernel counter dumps retain actual observations: absent
counters are not manufactured as zero or success.

`forwarding.sh` enters a private mount namespace sharing only E's existing
network namespace, makes the proc-sys bind writable there, writes the one
IPv4 forwarding key and requires a readback. Plain `sysctl -w` is insufficient:
the pinned tool can exit zero after reporting a read-only filesystem error.

Every phase begins `notRun`. Errors preserve the failing phase, daemon/CLI and
controller logs, namespace/RPDB/nft/conntrack snapshots, bounded pcap and source/
binary/image hashes. Missing phases, unsuccessful actor exits or cleanup errors
prevent a row pass. Captures and actors are joined before exact labelled
container/network removal; matrix and remote supervisor independently check
leftovers. No artifact inspection retroactively changes a run verdict.
