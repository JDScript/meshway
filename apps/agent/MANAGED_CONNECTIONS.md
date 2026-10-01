# Managed connections: agent integration handoff

Build `meshway-agent --features managed-connections` (includes `transport-lab`).
Run:

```sh
meshway-agent run --state /private/agent.json --apply-connections \
  --interval-seconds 5 --connection-lease-seconds 45 \
  --connection-profiles /private/profiles.json
```

`--apply-connections` implies `--connections`. Existing `run --connections` remains
saved-only. Profiles are optional for native UDP/TCP. The poll interval must be
strictly below the lease (lease range 10–300 seconds, default 45). No cached plan
starts before a successful fresh controller sync. Runtime reports are separate
from saved ACK receipts, which continue to contain `applied:false`.

## Private profile schema (frozen for integration)

Strict JSON, max 128 KiB, max 128 profiles, unique IDs; unknown fields rejected.
The JSON file must be a regular, non-symlink, current-user-owned 0600 file. Referenced
paths resolve relative to its parent, not the process working directory.
Read-only Docker mounts are supported for profiles and PEM inputs.

```json
{
  "version": 1,
  "profiles": [
    {
      "id": "lax-vless-server",
      "role": "server",
      "uuid": "11111111-1111-4111-8111-111111111111",
      "certificate": "lax-cert.pem",
      "private_key": "lax-key.pem"
    },
    {
      "id": "lax-vless-client",
      "role": "client",
      "uuid": "11111111-1111-4111-8111-111111111111",
      "ca_certificate": "ca.pem",
      "server_name": "lax.mesh.test"
    }
  ]
}
```

Use the server ID as the listener's `credential_ref`, and the client ID as the
outgoing candidate's `credential_ref`. UUIDs are VLESS wire credentials; Meshway
Noise still pins the full node ID and its bound public key. TLS uses explicit CA
roots and server-name verification, with no insecure option. Key PEM files require
0600; certificate/CA PEM files may be 0644, current-user-owned, with no group/other
write. Each PEM is bounded to 256 KiB. Managed VLESS reserves CONNECT target
`127.0.0.1:8421` as a wire marker only; the acceptor never proxies that destination.

## Managed Linux IPv4 dataplane

```sh
meshway-agent run --state /private/agent.json --apply-dataplane \
  --tun-name meshway0 --tun-mtu 1000 \
  --interval-seconds 5 --connection-lease-seconds 45 \
  --connection-profiles /private/profiles.json
```

`--apply-dataplane` explicitly opts into Linux IPv4 packet application and implies
managed adjacencies. It uses the enrolled identity and existing connection key;
there is no transport-lab daemon/configuration. Requires `managed-connections`,
`/dev/net/tun`, iproute2 and `CAP_NET_ADMIN`. A Docker namespace with read-only
`/proc/sys` must set `net.ipv6.conf.default.disable_ipv6=1` before creation (and
normally `net.ipv6.conf.all.disable_ipv6=1`). The agent configures no IPv6 addresses
or routes, kernel forwarding, NAT or Internet exit.

Kernels that omit the new interface's IPv6 sysctl are supported only with verified
`default.disable_ipv6=1` inheritance (or no IPv6 subsystem). The enabled interface
is also checked for IPv6-address absence before readiness. Permission errors,
read-only writes to an enabled interface, and unverified missing sysctls fail closed.

The generated `get_agent_dataplane()` operation supplies one digest-validated
global revision of configuration, adjacency, explicit directional access, and
public identity bindings. The private `agent.json.dataplane.json` cache is bound
to controller, node and credential hash. It supplies a rollback floor, never
offline startup authority. Fresh snapshots are required on every restart. Separate
configuration/connections saved ACKs use projections of this exact bundle; their
digests are also used by the unchanged adjacency runtime-report API.

Every packet path is the intersection of the routing compiler and an exact full
directional grant. Missing grants, first-match rejects, missing reverse grants,
and unknown relevant domain conditions deny packets. A reverse response needs its
own route and grant. E2E-only public keys receive `transports: []`; neither a grant
nor an endpoint key authorizes a physical carrier or listener. All traffic uses
the existing Runtime's authenticated E2E packet API, including at relays. Endpoints
check the full path, authenticated source ID, assigned source/destination IPs,
IPv4 header checksum and exact length. Fragments, IP options, IPv6 and addresses
outside an authorized assigned pair are dropped. Grants cover all IPv4 protocol
numbers; there is no implicit port ACL.

The default MTU is 1000 (minimum 576). Every compiled path is checked with
`native_udp_datagram_len` using full 64-character IDs plus 48 outer IPv6/UDP header
bytes against an outer MTU of 1500. An unsafe explicit MTU fails rather than
fragmenting or silently lowering it; eight-node paths need a smaller MTU. Underlay
transport targets, controller addresses and configured DNS servers cannot overlap
the mesh IPv4 prefix.

An unnamed, nonpersistent TUN is exclusively allocated, then atomically renamed;
an existing requested name is never adopted or deleted. It owns only the registered
IPv4 `/32` and the mesh-prefix route (metric 42759, protocol 242). A daemon-lifetime
TUN remains as a bounded drop sink when packet generations are inactive; its kernel
queue is discarded before a fresh generation. Changed bundle revisions conservatively
restart the complete runtime **before disk writes or HTTP ACKs**. Unchanged bundles
can renew only from their original fresh-fetch timestamp. The independent watchdog
cancels packet reads, blocked writes and E2E setup on expiry; runtime shutdown erases
old session caches. Resource failure withdraws the complete generation.

A validated identity publication newer than the installed generation also withdraws
that generation before the next snapshot GET. Either saved ACK returning 409 withdraws
it before retrying the GET, even when the fetched bundle matched the old installation.
Publication and stale-revision evidence never renew the packet lease; a stalled retry
cannot retain grants already known to be stale.

Endpoint DNS failure/timeouts during route-sink preparation are retryable, including
startup before a peer's service name exists. The old generation is withdrawn, the
owned sink/guard is retained, and retry starts with a new controller fetch. No failed
preparation renews a lease. A resolved endpoint inside the mesh prefix, changed TUN
binding, or filesystem/permission failure remains fatal.

Terminal controller HTTP status is surfaced before reading its response body. A
401/403 with an incomplete body stops the managed loop and withdraws active packet
resources immediately; it does not wait for the HTTP timeout or packet lease.

An exact-prefix `unreachable` guard (metric 42760, protocol 242) is deliberately
retained after interface removal, clean shutdown or process death. This prevents
the host's default underlay route from carrying Mesh IPs. Restart reuses only the
exact guard when the private bound dataplane cache exists. Any conflicting route
fails startup; routes are never replaced. Decommissioning a node includes explicitly
removing this guard after stopping its daemon, for example for a `100.64.0.0/24` pool:

```sh
ip -4 route del unreachable 100.64.0.0/24 metric 42760 proto 242
```

Local JSON events (`event: "managed_dataplane"`) expose actual `tun_ready`, node/IP,
MTU, installed generation revision/canonical dataplane digest, allowed-path count
and withdrawal reason. Withdrawal retains the withdrawn generation's digest, even
when newer saved ACKs have advanced. Ready means the owned configured
interface and packet generation are available; successful local enqueue is not a
remote delivery proof. HTTP `runtimeApplied` remains adjacency-only. Normal OS
`ping` and `curl` to assigned Mesh IPv4 addresses exercise the packet plane; the
private `meshway-agent ping` IPC command still performs authenticated neighbor probes.

## Adjacency-only runtime semantics

- With `--apply-connections`, only authenticated neighbor-control probes are applied;
  every adjacency-only core configuration has `allowed_paths: []`.
- Full 64-hex controller node IDs and the existing bound connection-key sidecar.
- Listener-scoped peer allowlists; native UDP/TCP and TLS + VLESS + pinned Noise.
- Endpoint DNS resolution uses the underlay with a deadline and bounded results;
  service names such as `lax.mesh.test` are supported and reevaluated every sync.
  Results are filtered/deduplicated and capped at eight per candidate; native
  resolved-address candidates have private local labels mapped back to the exact
  controller candidate ID in reports. DNS failures use `error/resolve_failed`.
  Converged native DNS/IP aliases are deduplicated by peer, carrier and resolved
  socket before core limits/validation. Priority then candidate ID selects the
  retained label; fully shadowed approved candidates remain waiting standbys.
- Semantic intent, loaded TLS/profile material and resolved target changes rebuild;
  global revision-only changes retain healthy channels. Material changes withdraw
  the previous runtime before installing the replacement. Invalid/missing profiles
  or failed DNS/binds report an empty-resource error, not applied.
- Fresh config sync alone renews the local lease. HTTP stalls/backoff and report
  success cannot extend it. Expiry/revocation/permanent state failure/shutdown close
  all sockets. Expiry is `withdrawn/lease_expired`. The startup runtime epoch is
  read once and pinned with the process nonce across begin retries; begin/report
  epoch conflicts stop the process without reacquiring.
- A validated, durably saved control sync resets outage retry backoff before
  reporting. A newly rebuilt generation's first probe may fence its report; that
  report retry must not reuse an old outage delay longer than the config lease.
  Recovery retains the process epoch and same semantic configuration, advancing
  the report sequence after actual neighbor probes resume.
- A neighbor is reported authenticated only after its recent nonce-control probe,
  with `reason:none`. TLS/VLESS dial failures retain sanitized authentication or
  transport reasons. Failed probes become unreachable/probe_failed. Native core
  has no per-attempt error API: an unestablished native candidate becomes
  unreachable/transport_failed after the initial five-second wait; standby
  candidates remain waiting. No specific native authentication error is inferred.
- Authenticated metadata/RTT comes from core's atomic
  `probe_neighbor_observed` result. Per-generation observation/version watches
  fence report construction and cancel/recheck the HTTP wait on changed proofs,
  withdrawal or lease expiry; reporting itself never refreshes configuration.
- Native capabilities advertise the existing three-carrier catalog only.

Core runtime resource ceilings remain enforced (64 trusted peers, 8 native
listeners, 32 distinct resolved native targets, 32 simultaneous channels/setup
attempts). Plans exceeding these bounds fail closed with invalid_configuration.

## Verification / ownership

Agent-owned implementation is in `src/managed_connections` and
`src/state/connections/managed.rs`. Main owns generated SDKs and Cargo.lock.
Checks: `cargo test -p meshway-agent --features managed-connections --locked`,
`cargo clippy -p meshway-agent --features managed-connections --all-targets --locked -- -D warnings`,
plus the ordinary non-feature Agent suite. Unit fixtures use real loopback UDP,
TCP and TLS/VLESS with full 64-hex node IDs and in-process rcgen-generated temporary
test certificates (fresh CA/key per fixture). Controller subprocess tests cover saved-vs-applied reporting,
durable key reuse, grant removal, stalled HTTP lease expiry, epoch supersession,
lost-begin CAS races, revocation, shutdown, and recovery of the same epoch/revision
after controller downtime grows retry backoff beyond the config lease. Docker
validation is coordinated separately by the main session.

Initial MAIN verification: 49 feature-enabled Agent tests and 31 ordinary Agent
tests passed; strict Clippy (all targets, managed feature) and package formatting
passed. The lost-begin regression checks A0→1, B1→2, then retry A0 is fenced,
with exactly one startup epoch read by A and B preserved at epoch 2/sequence 0.

Follow-up recovery/dedup verification: the real-controller long-outage regression
failed before the fix, with both agents retaining a 30-second retry after fenced
reports while their configuration leases were only 10 seconds. It passes after
resetting backoff on durable control sync: the same epoch/revision/digest recovers,
sequences advance, actual listeners return and fresh neighbor probes succeed.
The five retained controller roundtrips, ten managed runtime unit fixtures,
strict Clippy and formatting also pass. Native alias regressions cover UDP and
TCP, preferred metadata, waiting standbys, unrelated listeners, no-flap resync,
dedup before core count limits, and distinct peer/carrier boundaries.

Focused recovery command (includes a real outage long enough to reach 30s backoff):

```sh
cargo test -p meshway-agent --features managed-connections --locked \
  --test connections_roundtrip \
  same_epoch_configuration_recovers_after_outage_backoff_exceeds_lease
```
