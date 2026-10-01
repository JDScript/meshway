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

## Runtime semantics

- Only authenticated neighbor-control probes, no TUN/IP/transit/exit application;
  every managed core configuration has `allowed_paths: []`.
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
TCP and TLS/VLESS with full 64-hex node IDs and OpenSSL-generated temporary test
certificates. Controller subprocess tests cover saved-vs-applied reporting,
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
