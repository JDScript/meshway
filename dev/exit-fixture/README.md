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
            Resolver             |  \-- router --far-- the service's far address (router side MTU 1280)
                                 |lan ------ RFC1918 service
   (A's OS DNS upstream and useWithExit split resolver)
```

## Configuration and isolation

Nine labelled internal bridges have no published ports or Internet route.
A/E share neither a control nor a data link. Only E (and the forwarding-only
router) attaches to the service networks. Six RFC1918 links use Docker IPAM;
the external and far links use distinct random `11.77.x.0/29` subnets entirely
inside their internal bridges. This address is eligible for
Policy's Internet classification. The accepted permit must explicitly carry
`ExitDestinationScope::Internet`; TEST-NET/RFC1918 cannot satisfy that assertion.
Allocation conflicts fail without changing another network.

`controller/exit.rs` submits configuration through the generated management
client: only E has `exitEnabled`; only A is assigned the `exit` profile, whose
fallback is Exit to E via R. R/E keep `mesh`. Access is source A to
`autogroup:internet`, the exact allocated LAN CIDR and E itself (phase 16's
Mesh baseline, routed via R), limited to TCP 8080/5201, UDP 8080/8081 and ICMP.
Demand-7 topology permits A–R and R–E with Native UDP/TCP
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
capacity refusal (one flow refused, role kept), the exit recovery of OPS8 (a
re-created echo conntrack entry is no conflict; a deleted exit-owned route
stops only the role, which reinstalls while the Host keeps its resources),
the nft transaction and the live exit-prerequisite probe in separate
namespaces.
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
| 8 Drain/lease | Role withdrawal and old socket refusal; Node HTTP closed (in-flight sync polls ended, new requests held): `lease.py` keeps a TCP echo and new UDP flows working offline until the first path lease expiry, then they end; E withdraws its role at its own expiry with exact nft/conntrack cleanup and A keeps its negative egress gate; reopened HTTP recovers a new lease and new flows without a second up; SIGKILL/same-netns journal recovery, explicit down and foreign sentinel preservation | `withdrawn.json`, `lease-window.json`, `lease-*.json`, `expired.json`, `recovered.json`, `*crash*.json`, `down.json`, `foreign-sentinel-*.json` |
| 10 Domain exit | Runs after phase 9's exchange checks with a temporary profile: a name rule sends `*.name.exit-dns.test` to E via R, the fallback is direct and Access grants `domain:name.exit-dns.test`. The plan captures no name-zone address; a connection to the service address without the name follows the IP rules (direct, unreachable) and the service sees nothing; the first `getaddrinfo` of a name returns the service address in one attempt (E's answer, held until the signed association applied) and a TCP echo by name reaches the service from E's address only; at least one association task completion (controller count); a second connection by name adds none; while associated the bare address follows the name rule; after the `domain:` grant is revoked the named connection fails; the original configuration is restored before phase 5 | `domain-exit.json`, `domain-exit-configured.json`, `external/access.json`, `controller/association-reports.json` |
| 12 ICMP errors | A's `traceroute -I` shows E, the router and the far target; a 64 KiB TCP echo converges through the router's fragmentation needed (A caches `mtu 1280`, `ping -M do` 1300 fails, 1252 passes); router-injected time exceeded quoting A's live UDP flow (post-NAT) reach A within the per-flow budget, ones quoting no flow never do. E's far route is removed afterwards | `icmp-errors.json` |
| 13 Ping | A's real `meshway ping` (typed `--json` events): the external service's IP decides exit E on A→R→E (profile fallback) and its echo replies return on that path; the split resolver decides direct by rule 0 (its rule-set entry) and replies from A's own network; in phase 10 a name-rule name resolves through E's resolver with its live association, decides exit E by rule 2 and replies through E; after the `domain:` grant is revoked the same name decides denied and nothing is probed. Human output is kept per target | `ping-external.json`, `a/ping-*.json`, `a/ping-*.txt`, `domain-exit.json` |
| 14 Auto profile | Runs after phase 11 with a temporary configuration: `exit` and `exit-alt` gain a name rule (`*.name.exit-dns.test` → E via R) and a direct fallback; `auto` maps US → `exit`, JP → `exit-alt`, falls back to `mesh` and is A's default; the controller writes A's `region` as absent, US, JP, FR. `meshway exit auto` resolves to `mesh`/`noTrustedRegion` and nothing reaches the service; US resolves to `exit`/`label` and a resolved name and its bare address reach the service from E only; a TCP/UDP flow by name established under US fails entirely once JP (`exit-alt`/`label`) applies, the bare address no longer reaches the service and leaves A's capture after the flow, a new name leaves through E; `meshway exit exit` overrides auto and `meshway exit auto` returns to JP; FR resolves to `mesh`/`noRuleMatched`; from a manual `exit`, `meshway exit --default` clears the selection (no requested profile or version) and the default `auto` resolves to `mesh`/`noRuleMatched` again. The Server's `routing.autoResolve` activities (system actor) are exactly `mesh`, `exit`, `exit-alt`, `exit-alt`, `mesh`, `mesh`. A selects `exit` and the original configuration is restored before phase 5 | `auto-profile.json`, `controller/auto-activities.json` |
| 15 Echo capacity and recovery | After phase 12 (`echo.py`). A pings the external service every 0.2 s for 60 s while it repeats `traceroute -I`: E's exit projections peak at most 32 (a quarter of the owner's 128), every echo returns, E's role stays installed. E then deletes the conntrack entry of A's echo id twice while A pings (as its ICMP timeout does while earlier sequences are projected): the role stays installed and E logs no owner stop. Last, a return route of E's exit role is deleted under A's ping: E logs `Exit role failed; withdrawing it alone` and reinstalls the role within 25 s while staying running, applied and connected, and A's MagicDNS resolves every 0.5 s. Ordinary names (through E's resolver) may fail only in lookups overlapping the role's outage as E's own timestamps bound it (its role-stop log line to the acknowledgement of the first State report with the reinstalled role ready, plus 1 s for A to receive it), at most `ceil(window / 0.5 s) + 1` times: E reports itself not ready and A refusing its resolver on that current fact is fail closed. Every other lookup answers (`recovery.dnsOutage`) | `echo-capacity.json`, `components/recovery.log` |
| 16 Exit throughput and queue pressure | After phase 15 (`throughput.py`). iperf3 TCP from A for 20 s each: to E's Mesh address over the same A→R→E channels (Access `tcp:5201`, Mesh route via R) and to the external service through E, forward and reverse (`-R`); receiver rate, sender retransmissions and E/A interface, SNMP and qdisc counters are recorded, not asserted. Then one exit TCP flow for 20 s while A pings the service every 0.2 s (each echo a new exit flow, installed beside it): every 1 s interval carries data, at least 95 % of echoes return and E's role stays installed. Diagnostics throughout the phase, never asserted: captures of E's exit TUN and A's Mesh TUN (A without the Mesh iperf3, `-s 128`, ≤ 600,000 packets per file), 1 s samples on A and E (TUN counters, Status summary, E's conntrack count and `Exit queue full` count, A's iperf3 sockets) and the daemons' log events per second; a stuck iperf3 records both ends' sockets, E's conntrack and Status | `exit-throughput.json`, `*/phase16-*` |
| 9 DNS | Exchange checks run after phase 4, lifecycle checks inside phase 8; the phase passes after 8. A's plan is `viaExit` on A→R→E and E serves exactly A; system `getaddrinfo` on A returns E's answers (A, AAAA, 90-record answer that only fits over the Mesh TCP route) and the external authority's only asker is E's external address; A's own MagicDNS name and an unknown one stay local; the split name reaches the Resolver from A's split address only; with E's authority route blocked A's query fails and the Resolver never sees it; DNS fails on role withdrawal and lease expiry, works offline within the lease and recovers; `exit-dns` bytes appear only in E's external capture | `dns-exit.json`, `dns-lifecycle.json`, `external/dns-queries.json`, `resolver/dns-queries.json`, `*/*.pcap` |

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
