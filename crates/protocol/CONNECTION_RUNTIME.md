# Managed connection runtime contract (frozen handoff)

This is a separate self-report API. `NodeConnections` schema 1, its canonical
digest, `ConnectionNode`, and saved-only connection ACKs are unchanged.
All objects are closed; nullable **scalar** fields are required JSON null when
unknown. All successful requests return 200. No client timestamps, endpoints,
profile contents, secrets, traffic counters or IP-flow authorization are accepted.

## Operations and exact DTOs

| operationId | method/path | bearer | request → response |
| --- | --- | --- | --- |
| getConnectionRuntimeEpoch | GET `/api/v1/agent/connections/runtime` | node | → GetConnectionRuntimeEpochResponse |
| beginConnectionRuntime | POST `/api/v1/agent/connections/runtime/begin` | node | BeginConnectionRuntimeRequest → BeginConnectionRuntimeResponse |
| reportConnectionRuntime | POST `/api/v1/agent/connections/runtime` | node | ReportConnectionRuntimeRequest → ReportConnectionRuntimeResponse |
| getConnectionRuntimes | GET `/api/v1/connections/runtime` | admin | → ConnectionRuntimesSnapshot |

```text
GetConnectionRuntimeEpochResponse { epoch: u32 } // 0..u32::MAX, 0 means no lease
BeginConnectionRuntimeRequest { instance_id: string, expected_epoch: u32 }
  // instance_id: 64 lowercase hex, random per process; expected_epoch: 0..u32::MAX
BeginConnectionRuntimeResponse { epoch: u32 } // 1..u32::MAX
ConnectionRuntimeState = awaiting_report | disabled | applying | applied | error | withdrawn
ConnectionRuntimeReason = none | disabled | invalid_configuration | profile_unavailable |
  bind_failed | resolve_failed | transport_failed | authentication_failed | probe_failed |
  lease_superseded | lease_expired | shutting_down | internal_error
ConnectionListenerState = bound | error
ConnectionNeighborState = authenticated | waiting | unreachable
ConnectionListenerObservation { listener_id: string, state: ConnectionListenerState,
  reason: ConnectionRuntimeReason }
ConnectionNeighborObservation { peer_node_id: string, transport: ConnectionTransport,
  initiator_node_id: string, listener_id: string|null, candidate_id: string|null,
  state: ConnectionNeighborState, reason: ConnectionRuntimeReason,
  probe_fresh: bool, probe_rtt_ms: u32|null }
ReportConnectionRuntimeRequest { epoch: u32, sequence: u32, revision: u32,
  digest: string, state: ConnectionRuntimeState, reason: ConnectionRuntimeReason,
  listeners: ConnectionListenerObservation[], peers: ConnectionNeighborObservation[] }
ReportConnectionRuntimeResponse { epoch: u32, sequence: u32, reported_at: date-time }
ConnectionRuntimeObservation { node_id: string, epoch: u32, sequence: u32,
  revision: u32|null, digest: string|null, state: ConnectionRuntimeState,
  reason: ConnectionRuntimeReason, listeners: ConnectionListenerObservation[],
  peers: ConnectionNeighborObservation[], reported_at: date-time|null,
  stale_after_seconds: u32, current: bool, stale: bool }
ConnectionRuntimesSnapshot { nodes: ConnectionRuntimeObservation[] }
```

Acquired epoch, report sequence and report revision are 1..u32::MAX. Epoch-read
response and required begin expected_epoch allow 0..u32::MAX. Admin sequence is 0
before the first report; nullable revision/digest/time are null. Digests are 64
lowercase hex. Listeners max 128; peers max 256; RTT 0..u32::MAX milliseconds.
IDs max 64 ASCII `[A-Za-z0-9._-]+`. Report body max 128 KiB; begin max 8192 bytes.
`awaiting_report` is admin-only and rejected in reports.

## Lease, retries and freshness

- Active node bearer and previously bound connection public key are required.
  Node identity is derived exclusively from the bearer. Admin/node role mismatch
  is 401; revoked is 403; missing key is 422.
- Epoch GET uses the same active-node/bound-key guard and returns the current
  per-node epoch, or 0 if no lease exists. It does not create/renew a lease, change
  observations, or expose instance IDs. The returned epoch is a CAS input.
- Begin checks the current instance first: identical current instance returns its
  existing epoch and preserves report/time, even if expected_epoch is the original
  stale value. Otherwise expected_epoch must equal the stored epoch (or 0 without
  a lease); mismatch is 409 and leaves the lease and report untouched. Matching
  CAS increments the epoch and clears the report atomically. First acquisition
  therefore requires expected_epoch=0 and creates epoch 1. Admin then shows
  `awaiting_report`, sequence 0, no resources, current=false/stale=true.
  Epoch exhaustion fails atomically with 422; counters never wrap. At max epoch,
  same-instance retries still succeed; stale different-instance CAS returns 409.
- Agent reads the epoch once before its first begin attempt and **pins that
  expected_epoch and instance_id across all retries**, including timeout/lost
  response retries. Never refresh the epoch after a conflicting begin to steal
  ownership. A begin 409 means acquisition lost: stop/withdraw the process runtime.
  Example: A(expected=0)→1, B(expected=1)→2; delayed A(expected=0)→409, preserving
  B's report. Concurrent different instances with one expected epoch have one
  winner; delayed first requests cannot evict that winner.
- Reports from wrong epochs or older sequences return 409. Same sequence and
  canonical JSON body returns the original reported_at, without renewing freshness.
  Same sequence with changed body returns 409. Arrays are order-sensitive; object
  property order is not. New sequence can skip values but must increase.
- Config revision/digest mismatch returns 422, distinct from lease conflict.
  Invalid resource claims also return 422. Malformed/unknown JSON fields are 400.
  Current configuration is checked even on report retries.
- Agent serializes its sender. A superseded lease causes withdrawal/failure;
  an old process must not fight for ownership by calling begin again.
- Only nodes with a lease appear in the admin list, sorted by node ID. `current`
  requires active node and exact current node-scoped revision+digest; `stale` is
  true when !current, never reported, or server receive age >=60 seconds.
  `stale_after_seconds` is always 60. UI continues aging reported_at itself.
  Freshness is presentation metadata, never authorization or controller proof.
- New reports, lease changes and migration never bump global revision or write
  ACKs. Revocation clears the report; admin shows withdrawn/no resources with
  null report fields and current=false/stale=true. Historical config reports may
  retain resources, but are explicitly current=false/stale=true.

## State/resource validation and meaning

- `applied` means desired listeners/reconcile installed, not all peers connected.
  Every desired listener must appear bound with reason=none. Empty valid plans
  can be applied with empty lists. Peers need not all be authenticated or present.
- `applying` may contain a partial resource set. `disabled`, `withdrawn`, and
  global `error` require empty listener/peer lists. `applied`/`applying` require
  global reason=none; `error` requires non-none. Invalid plans allow only error
  or withdrawn, empty resources, reason=invalid_configuration.
- Listener IDs must be locally configured; bound requires reason=none, error
  requires non-none. Duplicate listener reports are rejected.
- Peer ID/carrier/listener/initiator must match the node-scoped adjacency intent.
  listener_id is required non-null for this slice. Outgoing initiator is self
  and candidate_id must identify an exact outgoing candidate. Incoming initiator
  is the peer, matched against incoming approvals; candidate_id may be null when
  the channel cannot distinguish configured dial candidates, or an exact approval
  ID. Duplicate (peer,carrier,initiator,listener,candidate) reports are rejected.
- authenticated requires `probe_fresh=true`: the Agent actually completed a
  recent authenticated neighbor control-nonce probe. RTT may be null if unknown.
  waiting/unreachable require probe_fresh=false and RTT=null. No handshake or
  control heartbeat alone can be reported authenticated. Controller validates
  scope but cannot independently prove the Agent's observation.
- Peer `reason` is required. authenticated requires none. waiting permits none
  (not yet attempted / awaiting incoming connection) or a peer-local failure:
  profile_unavailable, resolve_failed, transport_failed, authentication_failed,
  probe_failed, internal_error. unreachable requires one of those failure codes.
  Global lifecycle reasons and bind_failed are not peer-local reasons. A peer
  failure is compatible with node state=applied and global reason=none; it does
  not imply that installed listeners or the whole configuration failed. DNS
  failures use resolve_failed; connection/TLS transport failures use
  transport_failed; failed peer authentication uses authentication_failed.
  Raw exception strings, TLS details, URLs and profile contents are never sent.
- lease_expired describes expiry of the locally enforced native configuration
  lease, distinct from lease_superseded (another process owns the reporting
  epoch). Report lease expiry as withdrawn/error with empty resource lists.
- Report only after pulling fresh configuration while holding the lease. Runtime
  uses adjacent peer keys and empty allowed_paths; channels grant no business
  traffic. Local profile failures use reason codes, never profile contents.

## Integration ownership

Controller owns Rust DTOs/routes/storage/schema-v4 additive migration and Rust
generator node-operation selection. Main regenerates clients and checks drift;
Agent/UI consume generated interfaces. Rust agent selection includes epoch GET,
begin and report, not admin getConnectionRuntimes. No capability catalog change.
