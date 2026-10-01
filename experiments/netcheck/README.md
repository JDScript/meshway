# M2 independent UDP netcheck / kernel NAT acceptance

This fixture exercises the **real `meshway-agent netcheck` CLI** against two real
STUN observers through a Linux router container running IPv4 iptables SNAT.
Observers encode their actual UDP `recvfrom()` peer in RFC 8489 XOR-MAPPED-ADDRESS;
scenario settings never choose the address reported by an observer.

**Validation status:** all seven kernel-NAT scenarios passed on 2026-10-01,
run `45c3ba2b4d9a4acdb5d6972890c0094e`, including the held-socket unsolicited
canary and complete owned-resource cleanup. Image `meshway-netcheck:m2-20261001-a`
is native ARM64, immutable ID
`sha256:5a6a14dedebd2199f64fae00bc4400ef4d8b8584aca9bacd49c9ef1f9e59cd55`.
Agent source is the integrated `74a2bde` slice. The fixture also passed 27 host
tests. The host's retained artifact is `meshway-netcheck-dp18z2df/result.json`
under the approved OpenCode temporary directory.

## Host verification (no Docker)

From the workspace root:

```sh
python3 -B -m unittest discover -s experiments/netcheck -p 'test_*.py' -v
python3 -B experiments/netcheck/run.py --help
git diff --check
```

Tests use Python standard library, ephemeral **loopback-only** IPv4/IPv6 UDP
sockets, and private temporary directories. They cover real peer address
encoding, source guards, malformed packets, SOFTWARE/FINGERPRINT handling,
rate limits, held-socket control groups and nonce detection, strict report
validation, unknown results, single-socket conntrack evidence, native-image
preflight, ownership-checked cleanup and failure/interrupt artifacts.
IPv6 is skipped only if the host cannot bind `::1`; the Docker NAT fixture itself
is IPv4.

## Image build and execution — Main only

Main supplies a newly built **native Ubuntu/Debian** image containing the new
real Agent binary and uses this derived Dockerfile to add `python3`, `iproute2`,
`iptables`, and `conntrack`. This directory does not change any existing image,
demo, manifest, lockfile, or fixture. Build commands run under the existing
shared performance-lock wrapper:

```sh
MESHWAY_PERF_LOCK=/private/var/folders/8n/20x15sl9797cdg9sdqfj803w0000gn/T/opencode/meshway-performance.lock \
  python3 experiments/transport/performance_lock.py \
  docker build --file experiments/netcheck/Dockerfile.tools \
  --build-arg BASE_IMAGE=MAIN_FRESH_NATIVE_AGENT_IMAGE \
  --tag meshway-netcheck:m2-d4d5dec .
```

Replace `MAIN_FRESH_NATIVE_AGENT_IMAGE` with Main's freshly built base image.
The runner does not build or pull. It accepts only a dedicated
`meshway-netcheck:TAG`, resolves it to a full `sha256:…` immutable image ID, rejects
image volumes, and checks **host = Docker daemon = image native architecture**
and Linux image/daemon OS. Every container is created using that immutable ID.

Main runs this command through its long-run tmux wrapper:

```sh
python3 -B experiments/netcheck/run.py --image meshway-netcheck:m2-d4d5dec
```

`run.py` itself takes the same shared lock at the exact path above **before any
Docker command** and keeps it through all scenarios, ownership inspection,
resource removal and result writing. Do not wrap it in another lock acquisition.
The descriptor is inherited by each Docker subprocess. A SIGINT/SIGTERM starts
cleanup; further signals are ignored while cleanup finishes. SIGKILL cannot run
`finally`; Main's tmux wrapper should terminate gracefully.

The default artifact parent is the existing approved OpenCode directory:

```text
/private/var/folders/8n/20x15sl9797cdg9sdqfj803w0000gn/T/opencode
```

`--artifact-parent EXISTING_OWNED_DIRECTORY` can select an existing parent. A
fresh 0700 run directory contains an atomic 0600 `result.json`. The last stdout
line is a JSON summary with the result path, fixture status and cleanup status.
No raw Agent stderr, observer packets, transaction IDs/nonces, Docker output,
environment, user state directories or process logs are saved. Rejected external
output is never copied into the result. Failure messages are fixed fixture text.

## Isolation and evidence

Exactly four new containers and two new networks are created:

```text
client -- internal LAN bridge -- router -- internal WAN bridge -- observer A
                                                         \----- observer B
```

- Networks use unique names/run labels, `--internal`, bridge driver and Docker's
  automatic IPAM. No fixed CIDR, published port, host network or host mount.
- Client attaches only to LAN; A and B attach only to WAN with different own IPs;
  router attaches to both. Docker exec is the out-of-band coordinator.
- All containers: cap-drop ALL, read-only root, no-new-privileges, no logging
  driver, private 0700 `/private` tmpfs, 1 CPU, 256 MiB memory/no additional swap,
  64 PID limit, no privileged mode. Observers run as UID/GID 65532 with no added
  capabilities. Only client and router add NET_ADMIN.
- Helper code is written through stdin by each container's configured UID into
  its owned private tmpfs; this also works when Docker refuses `cp` into a
  read-only rootfs. Capability inspection accepts Docker's `CAP_NET_ADMIN`
  spelling while retaining the exact NET_ADMIN-only allowlist.
- Client changes only its own default route to the owned router's LAN IP. Router
  enables only its own namespace's `net.ipv4.ip_forward=1` via Docker's namespaced
  `--sysctl` and changes only its own iptables/conntrack tables. Each scenario
  flushes this router's NAT/filter rules and `conntrack -F` **inside the router**.
  No host route, DNS, firewall, daemon setting or existing service is changed.
- Observers reply to the router WAN IP, so they need no routing changes or
  NET_ADMIN. The WAN IP is the fixture's NAT-facing address, **not an Internet
  public address**.
- Each server accepts at most 512 bytes/request, checks the exact router WAN
  source IP inside its owned WAN subnet and rate-limits all input to 20 packets
  per second. Attribute-free 20-byte Binding requests, bounded SOFTWARE,
  correctly calculated final FINGERPRINT and unknown optional attributes work.
  Malformed/oversized requests and unsupported required/auth attributes receive
  no response. Only a bounded list of allowlisted actual peer endpoints is saved.
- Route-get proof must show both server destinations via the router LAN IP.
  Named router SNAT/forward/drop/return counters must increase as applicable.
  Successful original/reply conntrack tuples must match the Agent's declared
  single local socket and the measured WAN mapping. Observer peer sequences must
  match every successful A/B/A sample. These checks prevent a server fabricated
  address from passing as kernel NAT.
- After each scenario the client's private directory must contain no state.
  The filter helper's explicitly owned code/control files are removed before this
  check. No controller, registration token, node identity, key or listener state
  is provisioned.

Cleanup inspects exact created/attempted resource IDs/names and checks both run
and role labels before removal. Containers are removed before networks. A failed
inspection or ownership mismatch is preserved as a structured cleanup failure;
other resources are still attempted. There is no prune, broad label removal,
image deletion or anonymous volume deletion. Overall exit is nonzero if cleanup
is incomplete.

## Agent contract checked by this fixture

Each independent invocation is:

```text
meshway-agent netcheck \
  --stun-server <A_NUMERIC_IP>:3478 --stun-server <B_NUMERIC_IP>:3478 \
  --bind 0.0.0.0:0 --timeout-ms 700 --json
```

One temporary UDP socket probes **A, B, A** in order. Stdout must contain exactly
one strict JSON report (no logs, duplicate keys or nonfinite numbers):

```text
schema_version: 1
source: "independent_udp_probe"
measured_at / completed_at / valid_until: timezone-bearing RFC3339 strings
bound_socket: numeric IPv4/IPv6 endpoint including actual nonzero local port
address_family: "ipv4" | "ipv6"
udp_observed: boolean
mapping_varies_by_destination: boolean | null
mapping_stable_during_check: boolean | null
samples: [{ server, outcome, mapped_endpoint, rtt_ms, elapsed_ms }, ...]
outcome: mapped | timeout | invalid_response | io_error | stun_error
```

The report must describe this actual invocation. Expiry must be after completion
and at most 30 seconds from measurement. Mapped samples have numeric endpoints,
finite nonnegative RTT/elapsed times; failed samples have null endpoint and RTT.
Exit 0 means at least one valid observation, exit 1 means none, exit 2 is argument
failure (Agent parameter tests belong to the Agent slice). This fixture requires
two different observer IPs; the Agent's maximum-four-server validation remains
with its own unit tests.

Only complete, stable A/B/A observations can make a destination comparison.
Partial/failure/changing-A comparison is null, never silently false. In the
partial scenario, A-repeat stability can be true, or null if the Agent gates it on
the complete check; destination variation must be null either way. No sample
means `udp_observed=false` for **this check**, not proof of global UDP blocking or
absence of network connectivity.

This is a fixture-side runtime validator of the CLI contract, not a new HTTP DTO,
generated client, authoritative product schema or protocol implementation. Main
can adjust minor contract details here when integrating the Agent slice.

## Verified scenario results

`W` below is the actual dynamically allocated router WAN IP.

| Scenario | Router setting | Measured A, B, A | UDP | Varies | Stable | CLI exit |
| --- | --- | --- | --- | --- | --- | --- |
| `stable_snat` | SNAT both destinations to W:41000 | W:41000, W:41000, W:41000 | true | false | true | 0 |
| `destination_dependent_mapping` | A→41001, B→41002 | W:41001, W:41002, W:41001 | true | true | true | 0 |
| `endpoint_restricted_fixture` | SNAT 41000; default DROP, allow LAN requests and established return | W:41000, W:41000, W:41000 | true | false | true | 0 |
| `udp_drop` | Drop client UDP before SNAT, count drops | timeout, timeout, timeout | false | null | null | 1 |
| `partial` | Drop only client→B; SNAT A 41000 | W:41000, timeout, W:41000 | true | null | true or null | 0 |
| `mapping_change_before` | Fresh CLI, SNAT 41003 | W:41003, W:41003, W:41003 | true | false | true | 0 |
| `mapping_change_after` | Flush own conntrack, fresh CLI, SNAT 41004 | W:41004, W:41004, W:41004 | true | false | true | 0 |

### Held-socket unsolicited canary

`endpoint_restricted_fixture` has an additional **independent filter helper**
after the CLI measurement. It is not a netcheck report field or classification.

1. Clear only the owned router's old conntrack entry so the CLI's closed socket
   does not reserve the fixed 41000 mapping.
2. A helper binds a new client UDP socket and keeps it **open**. It performs a
   real Binding exchange with A, observes W:41000 and writes a private readiness
   snapshot. B has never been contacted by this held socket.
3. Main's fixture coordinator reads readiness with Docker exec. B binds its own
   WAN IP on port 3479 and sends a unique recognisable nonce to W:41000.
4. A private sent-marker starts the client's bounded 700 ms receive window. The
   client detects the exact nonce/source if received. The router's explicitly
   tagged INPUT drop counter must increase: this unconnected B tuple cannot be
   reverse-SNATed as the established A reply. Default FORWARD DROP also applies.
5. On the **same still-open socket**, the helper performs another real A Binding
   exchange. Both legitimate control responses and actual A observer peers must
   match W:41000; the nonce must not have been received. Only then is the helper
   evidence accepted and the socket closed.

The artifact records the held socket, both controls, bounded window, nonce
absence and actual router drop-counter evidence. This proves the configured
fixture rejected that specific unsolicited packet while legitimate A replies
worked. It does **not** infer an Internet NAT filtering taxonomy or claim that
netcheck detected a filtering type. Host tests also verify the helper recognises
a nonce that *does* arrive, and missing send coordination fails rather than
becoming a false filtered success.

### Mapping changes and freshness

The last two rows are separate actual CLI invocations around a router SNAT
change. Their `measured_at` values must differ, each mapping must match its current
kernel setting, and each validity window is bounded. A saved report is a dated
observation: even before `valid_until`, a changed mapping invalidates reuse as
live reachability evidence. Time variation within one A/B/A check is covered by
host validator tests and the Agent slice's unit tests; there is no claim of an
in-check kernel remap test here.

## Result boundaries

`result.json` separates **settings**, the actual Agent **measurement**, and
independent **evidence** for each passed scenario. On failure it retains passed
scenarios, the fixed stage/check reason, and final cleanup status. Top-level exit:
0 all scenarios and cleanup pass; 1 fixture/check/cleanup failure; 130 graceful
interruption with complete cleanup. Individual all-timeout CLI exit 1 is an
expected successful acceptance case.

Observed destination-dependent mapping is not a “symmetric NAT” classification.
The source is `independent_udp_probe`: it is not an observation of a running
daemon, a listener, or a data carrier. The temporary CLI socket closes on exit;
its mapped endpoint cannot be automatically registered or used as a current
carrier candidate. Carrier-bound probing, controller report ingestion, candidate
registration/leases, rendezvous and actual hole punching are subsequent M2 work
and are **not implemented by this fixture**.
