# Isolated IPv4 exit acceptance

`exit/off` runs the real Server, three independent daemons/CLI instances and two
external application services. It remains **explicit-only until two full runs
pass**. A short run is diagnostic evidence; a component pass is not a row pass.
Current acceptance status is defined in `docs/v2/node/validation.md`.

```text
              Controller (forwarding=0)
               |ca      |cr       |ce
               A --ar-- R --re-- E
                                |external -- isolated public-classified service
                                |lan ------ RFC1918 service
```

## Configuration and isolation

Seven labelled internal bridges have no published ports or Internet route.
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

MagicDNS/overrideLocal are false and DNS servers/default/routes/searchDomains
are empty. Every Node still runs the existing private D-Bus/resolved namespace
wrapper, since compiled exit DNS can require a negative root installation.
Actual compiled DNS plans and independent IPC exit/DNS facts are retained.

Containers use the node-lab digest-locked OS image with manifest-checked,
read-only host-built executables. Each actor has a read-only root, private
state, 2 CPU/768 MiB/128 PID limits, no-new-privileges and dropped capabilities.
A/R/E receive TUN/NET_ADMIN plus the existing resolved namespace capabilities.
No host network, privileged container, Docker socket mount or host VM access
is used. All local mutations are in fixture-owned namespaces.

## Invocation

```sh
mise run validate:matrix --rows exit/off --short
git push planck feat/exit-acceptance
mise run validate:remote -- --ref feat/exit-acceptance --rows exit/off
```

The shared runtime build includes the production daemon library test binary.
`components.py` runs X4's real driver, handoff, role, DNS handoff and
forwarding-disabled cases in separate namespaces against an isolated UDP echo
service. Their results are labelled `scope=component`; the main actors still
must prove their own installation, NAT and traffic lifecycle.

## Assertion and evidence boundaries

| Phase | Assertions | Main evidence |
| --- | --- | --- |
| 1 Capability | Accepted acquire declares `feature:exit-ipv4` v1; capability instance equals live instance | `e/acquisition.json`, `e/installed-status.json` |
| 2 Prerequisite | `ip_forward=0` gives `ipForwardDisabled`, empty installed families and no role; restore reads back 1 | `ip-forward-disabled.json`, `forwarding-restored.json` |
| 3 Applied | Ready IPv4 role, exact saved/applied reference, real kernel resources | `installed.json`, `e/installed-status.json` |
| 4 Capture/transit | A default capture routes the external IP to its actual TUN; R remains a non-exit with original external transit permits | `capture-before-admission.json`, `capture-route.json`, `original-path.json` |
| 5 Projection/NAT | Original and reverse conntrack tuple/owned mark match external service log; exact ACK/first-write/confirmation ordering is checked by the production driver test | `nat-proof.json`, `first-flow.json`, `components/driver.log` |
| 6 Renewal/failure | Persistent TCP/UDP and receive-only UDP; driver checks original cutoff, ConfirmFailed exact isolation and surviving flow | `a/traffic-*.json`, `a/receive-only-*.json`, `components/{driver,handoff}.log` |
| 7 Reply/path | Real TCP/UDP/echo, zero R TUN delivery and no payload in R pcap; driver asserts `SecureNode::exit_reply` and original reverse path | `original-path.json`, `r/*.pcap`, `icmp-*.json`, `business-block-control.json` |
| 8 Drain/recovery | Role withdrawal, old socket refusal, held Node HTTP through instance expiry, fresh instance recovery, SIGKILL/same-netns journal recovery, explicit down and foreign sentinel preservation | `withdrawn.json`, `expired.json`, `*crash*.json`, `down.json`, `foreign-sentinel-*.json` |

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
