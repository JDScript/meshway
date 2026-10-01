# M2 slice 1: independent UDP STUN netcheck

Implemented CLI: `meshway-agent netcheck`. Available without feature flags and
with `managed-connections`; no platform-specific cfg is needed. A caller supplies
1–4 explicit numeric STUN UDP endpoints. The current process owns one newly
bound, unconnected socket and uses `send_to`/`recv_from` throughout.

```sh
meshway-agent netcheck --stun-server 127.0.0.1:3478 --stun-server 127.0.0.1:3479 --bind 0.0.0.0:0 --timeout-ms 1000 --json
meshway-agent netcheck --stun-server '[::1]:3478' --bind '[::]:0' --timeout-ms 1000
```

Examples require local STUN fixtures on those ports. There are no public-server
defaults or DNS bootstrap. The command does not read agent state/private
credentials, contact a daemon/controller, publish candidates, or mutate
connections, routes, or DNS. It does not reuse an existing socket or enable
address/port reuse. An occupied explicit bind port fails.

## Input, sequence, bounds and exits

- `--stun-server <NUMERIC_IP:PORT>` is required and repeatable 1–4 times; duplicate
  endpoints are rejected. Loopback is allowed for local fixtures. Unspecified,
  multicast, IPv4 broadcast, port 0, hostnames, IPv4-mapped IPv6 and IPv6 scope
  IDs are rejected. There is no implicit IPv6 scope resolution.
- `--bind` defaults to `0.0.0.0:0`; explicit IPv6 such as `[::]:0` is required for
  IPv6 observers. Bind and every observer must have the same family. Bind may
  use unspecified address, ephemeral port 0, or an explicit free unicast port.
- `--timeout-ms` defaults to 1000, range 250–3000. Each transaction has one
  absolute deadline including send/receive; no packet resets it. A single
  request is sent per transaction, without retransmission (bounded diagnostic
  usage, not the RFC's default long-lived retry schedule).
- Sequence is A, B, C, D as supplied, then A again when count ≥2. All requests
  keep the same local port and use new cryptographically random 96-bit IDs.
  At most five transactions, plus an independent 15-second overall deadline.
  The socket is closed before the report is returned/output.
- Receive storage is 1025 bytes, with the extra byte detecting datagrams larger
  than the 1024-byte parsing limit even when the OS truncates them. At most
  64 attributes are parsed. After each 32 ignored datagrams the receiver yields
  cooperatively; it also explicitly checks the deadline on each iteration.
- Exit **0**: at least one valid mapped response. Exit **1**: all probes failed,
  or socket bind/local setup failed, with diagnostics on stderr. Exit **2**:
  invalid arguments, before binding/network IO. `--json` produces exactly one
  single-line Report for every successfully bound check, including total probe
  failure. A bind/setup failure emits no stdout because no check exists.

## Exact closed local report contract

Public Rust types live in `meshway_agent::netcheck`. Report and Sample derive
`Serialize`/`Deserialize` with `deny_unknown_fields`; nullable fields must be
present (null is explicit), unknown enum values and schema versions are rejected.
This is a local CLI report, not an HTTP API/OpenAPI component.

| Report field | JSON type / meaning |
| --- | --- |
| `schema_version` | integer, exactly `1` |
| `source` | enum, exactly `"independent_udp_probe"` |
| `measured_at` | UTC RFC3339 timestamp, before bind/first observation |
| `completed_at` | UTC RFC3339 timestamp, after final probe |
| `valid_until` | UTC RFC3339 timestamp, `measured_at + 30 seconds` |
| `bound_socket` | string, actual numeric bound IP:port; wildcard stays wildcard |
| `address_family` | enum `"ipv4"` or `"ipv6"` |
| `udp_observed` | boolean, at least one valid mapped UDP response **in this check** |
| `mapping_varies_by_destination` | boolean or null; only tested destination IPs |
| `mapping_stable_during_check` | boolean or null; first A vs final A |
| `samples` | array of Sample, in actual transaction order, 1 or 3–5 entries |

| Sample field | JSON type / meaning |
| --- | --- |
| `server` | string, exact numeric observer IP:port |
| `outcome` | enum `"mapped"`, `"timeout"`, `"invalid_response"`, `"io_error"`, `"stun_error"` |
| `mapped_endpoint` | numeric IP:port string only on mapped success, otherwise null |
| `rtt_ms` | integer u32 only on mapped success, otherwise null; monotonic elapsed from transaction start |
| `elapsed_ms` | integer u32 for every outcome, monotonic transaction elapsed |

No additional fields are present. STUN reason phrases, arbitrary JSON, credentials
and node identity are not included. `stun_error` means a structurally valid
Binding error with ERROR-CODE; it is never a mapped endpoint. IO errors during
an individual transaction become `io_error`. Unsupported or malformed replies
can be followed by a valid reply before the same deadline; otherwise malformed
expected-server traffic yields `invalid_response`. Wrong source, cookie, class,
method or transaction ID is ignored and cannot produce mapped success.

## Comparisons and observation freshness

- A single observer yields both comparison fields null.
- First and final A must both map successfully to report stability. Equal
  endpoints give true; unequal endpoints give false; a missing observation gives
  null. This is evidence at the two sampled times, not proof of uninterrupted
  mapping stability.
- Destination variation is known only when A repeats successfully **and equally**,
  all requested observations succeed, and at least two observer IPs differ.
  Compare endpoint values across different destination IPs. Different ports on
  the same IP alone never establish false/true variation. A changed repeat,
  timeout, malformed reply or other missing required sample gives null.
- These fields do not classify a NAT type or establish general reachability.
  A false value only describes the tested destinations and observation window.
- `Report::is_fresh(now)` accepts completion ≤ now < valid_until, rejects future
  or reversed timestamps, windows extended past measured_at +30s, checks taking
  over 15s, and unsupported versions. It indicates observation freshness only.
  The socket is already closed, so even a fresh mapping is **not** a carrier
  candidate and cannot be presented as an active listener.

## RFC 8489 wire policy

Binding requests use the RFC header, cookie `0x2112a442`, and crypto-random ID.
Responses require exact Binding success/error class, zero high header bits,
matching cookie/ID, exact datagram/body length and four-byte aligned body.
Source IP **and port** must exactly match the current expected observer; no
alternate-server/redirection mechanism is followed.

XOR-MAPPED-ADDRESS decodes IPv4 with the cookie and IPv6 with cookie + transaction
ID. MAPPED-ADDRESS is a fallback only without XOR; when both appear they must
agree. Duplicate address attributes are rejected even when identical. Address
attribute lengths must be exact and mapped IP family must match the socket;
unspecified/multicast/broadcast/mapped-v6 addresses and zero ports are rejected.
Unknown comprehension-required attributes are rejected; well-formed optional
attributes are ignored. A present FINGERPRINT must be last, length 4, and match
CRC-32/IEEE XOR `0x5354554e`. MESSAGE-INTEGRITY/SHA256 attributes are explicitly
unsupported without credentials; the command never claims authentication.

STUN observation is neither Meshway peer identity nor hole punching. An IPv6
underlay measurement is not IPv6 mesh dataplane support. Real carrier socket
measurements, candidate leases, controller reporting and managed traversal are
future M2 slices.

## Local verification and coordinator handoff

```sh
cargo fmt --all -- --check
cargo test --locked -p meshway-agent --test netcheck
cargo test --locked -p meshway-agent --lib netcheck::tests
cargo test --locked -p meshway-agent --features managed-connections --test netcheck
cargo test --locked -p meshway-agent --features managed-connections --lib netcheck::tests
cargo clippy --locked -p meshway-agent --all-targets -- -D warnings
cargo clippy --locked -p meshway-agent --all-targets --features managed-connections -- -D warnings
```

Wire fixtures bind only IPv4/native IPv6 loopback, emit real RFC packets, and
run the actual CLI binary. IPv6 skips only explicit native-family/loopback
unavailability (`NoIpv6`), not arbitrary errors. macOS cannot bind `127.0.0.2`
without a host alias, so distinct destination-IP comparison is covered by pure
contract tests here; the coordinator's isolated NAT fixture should exercise
actual distinct observer IPs. No host route/DNS/firewall changes or public STUN
probes are needed. The coordinator owns root manifests/lockfiles, product docs,
and final isolated NAT verification.

Design references (read-only): RFC 8489, especially sections 5, 6.3, 14.1,
14.2 and 14.7; upstream Tailscale `net/stun/stun.go` and `net/netcheck/netcheck.go`
for diagnostic responsibilities. This codec was written directly for Meshway's
strict bounded usage, without copying upstream source blocks.
