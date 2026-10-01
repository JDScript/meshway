# Connection control-intent contract (schema 1, handoff)

This contract is frozen for the controller / generated agent / Web slice. Existing
`NodeSummary`, enrollment and `NodeConfiguration` remain unchanged. No runtime apply,
peer proof, live-channel claim, transit/exit grant or IP-flow grant is implemented.

## Operations

All objects are closed (`deny_unknown_fields`); all nullable fields below are
**required**, with explicit JSON null. All successful operations return HTTP 200.

| Operation ID | Method / path | Bearer | Request → response |
| --- | --- | --- | --- |
| publishConnectionIdentity | POST `/api/v1/agent/connection-identity` | node | PublishConnectionIdentityRequest → PublishConnectionIdentityResponse |
| getConnections | GET `/api/v1/connections` | admin | → ConnectionsSnapshot |
| saveConnections | POST `/api/v1/connections` | admin | SaveConnectionsRequest → ConnectionsSnapshot |
| getAgentConnections | GET `/api/v1/agent/connections` | node | → AgentConnectionsSnapshot |
| acknowledgeAgentConnections | POST `/api/v1/agent/connections/ack` | node | AcknowledgeAgentConnectionsRequest → AcknowledgeAgentConnectionsResponse |

- `ConnectionTransport`: `native_udp | native_tcp | vless`.
- `ConnectionSemantics`: `datagram | stream`.
- `ConnectionCapability`: `{transport, can_dial:bool, can_accept:bool, semantics}`.
  At most 3 entries, one per transport; UDP is datagram, TCP/VLESS stream; at least
  one role must be true. Advertise actual compiled implementations only. Sorted by
  transport (native_udp, native_tcp, vless). Capability does not enable a listener.
- `PublishConnectionIdentityRequest`: `{public_key:string, capabilities:ConnectionCapability[]}`.
  Key is canonical X25519, 64 lowercase hex; low-order keys rejected using Dalek.
  Caller identity comes exclusively from the node bearer. Immutable first binding;
  identical key is idempotent, a different key or another node's key is 409,
  including keys retained by revoked nodes. Same key can update capabilities.
- `PublishConnectionIdentityResponse`: `{revision:u32, node_id:string, public_key:string, capabilities:ConnectionCapability[]}`.
- `ConnectionEndpoint`: `{host:string, port:u16}`. Port 1..65535; canonical IP or
  lowercase IDNA DNS hostname (no trailing dot in stored form). No advertised
  wildcard, multicast, broadcast or invalid hostname; LAN and loopback permitted.
- `ConnectionListener`: `{id:string, node_id:string, transport, bind:string,
  advertised_endpoints:ConnectionEndpoint[], credential_ref:string|null}`.
  Bind is a socket address, nonzero port; unspecified bind is permitted. 1..16
  advertised endpoints, unique after normalization.
- `ConnectionCandidate`: `{id:string, dialer_node_id:string, listener_id:string,
  endpoint:ConnectionEndpoint, priority:u16, credential_ref:string|null}`.
  Priority is lower-first primary/backup intent (0..65535). Endpoint must equal an
  advertised endpoint after normalization. Acceptor comes from listener owner.
  Self-dial and duplicate (dialer, listener, endpoint) are invalid. Directed dial
  is independent of business direction; e.g. SHA may dial LAX for LAX→SHA routing.
- IDs and non-null credential refs: 1..64 ASCII `[A-Za-z0-9._-]+`.
  Native credential refs must be null. VLESS requires independently configured
  local-profile refs on listener and dialer; actual secrets stay local.
  VLESS uses reserved Meshway protocol target `127.0.0.1:8421`: this is a protocol
  marker, **not an actual proxy outbound target**. This slice does not resolve
  profiles, provision UUIDs/TLS or configure a proxy. Runtime apply is deferred.
- `ConnectionPlan`: `{listeners:ConnectionListener[], candidates:ConnectionCandidate[]}`.
  Initial empty plan is valid. Max 128 listeners / 256 candidates. IDs unique
  within each collection. Arrays stored sorted by ID; endpoints sorted host/port.
- `SaveConnectionsRequest`: `{expected_revision:u32, configuration:ConnectionPlan}`.
  Strict global CAS (409 even for stale no-op); validation errors 422. Semantic
  no-op saves do not bump revision. All key/capability/plan mutations share the
  existing global configuration revision.
- `ConnectionDiagnostic`: `{item_id:string|null, field:string, message:string}`.
- `ConnectionNode`: `{node:NodeSummary, public_key:string|null,
  capabilities:ConnectionCapability[], acknowledged_revision:u32,
  acknowledged_digest:string|null, acknowledged_at:date-time|null}`.
  Connection receipt fields are independent of `node.acknowledged_revision`.
  Initial connection receipt revision is 0, digest/time null. Receipt means saved
  only. Observation and receipt timestamps appear only in admin inventory.
- `ConnectionsSnapshot`: `{revision:u32, configuration:ConnectionPlan, valid:bool,
  diagnostics:ConnectionDiagnostic[], nodes:ConnectionNode[]}`. Includes revoked
  nodes and key tombstones; nodes sorted by ID. Invalid source is retained for
  repair (e.g. after revocation or capability withdrawal).

## Node-scoped publication

- `ConnectionPeer`: `{node_id:string, public_key:string}`.
- `OutgoingConnectionCandidate`: `{id:string, listener_id:string,
  peer_node_id:string, transport, endpoint:ConnectionEndpoint, priority:u16,
  credential_ref:string|null}`. Contains only this dialer's local profile ref.
- `IncomingConnectionApproval`: `{candidate_id:string, listener_id:string,
  peer_node_id:string, transport}`. No remote profile refs.
- `NodeConnections`: `{schema_version:u32, revision:u32, node_id:string,
  public_key:string|null, capabilities:ConnectionCapability[], valid:bool,
  diagnostics:ConnectionDiagnostic[], listeners:ConnectionListener[],
  outgoing:OutgoingConnectionCandidate[], incoming:IncomingConnectionApproval[],
  peers:ConnectionPeer[]}`. Schema version is 1. Only owned listeners, locally
  dialed candidates, incoming approvals, and their directly adjacent peers.
  Outgoing sorted (priority, id), incoming by candidate_id, peers by node_id.
  A configured candidate authorizes only that peer/carrier adjacency. A listener
  alone grants no adjacency. No routing path or runtime activation is implied.
  Invalid global plan publishes empty listeners/outgoing/incoming/peers to every
  node, valid=false, with bounded generic fail-closed diagnostics.
- `AgentConnectionsSnapshot`: `{digest:string, payload:NodeConnections}`.
  SHA-256 hex over existing `canonical_json(payload)` (sorted object keys, stable
  arrays); no observation/receipt timestamps in payload. Separate agent cache is
  required; this is not the IP/routing configuration cache.
- `AcknowledgeAgentConnectionsRequest`: `{revision:u32, digest:string}`.
- `AcknowledgeAgentConnectionsResponse`: `{revision:u32, digest:string,
  status:ConnectionReceiptStatus}` where status is always `saved`.
  Active authenticated node, exact current node-scoped revision/hash required;
  older/future/mismatched receipt is 409. Retry preserves receipt timestamp and
  never bumps revision. Existing IP/routing ACK does not acknowledge connections.

## Limits / persistence

Identity and ACK bodies: 8192 bytes. Save plan body: 128 KiB scoped to new route.
Agent snapshot: at most 1 MiB; publication bound checked transactionally with
mutations. DTO/JSON errors 400; revoked node 403; role mismatch 401. Capacity and
invalid plan errors 422. SQLite v2→v3 is additive; v1 runs existing IPAM upgrade
first. Preserve bootstrap token, credential/token bindings, IPs, policy, revision.

Shared Rust entry points: `connection_digest(&NodeConnections)`,
`PublishConnectionIdentityRequest::validate()`, `valid_connection_public_key(&str)`;
constants `CONNECTIONS_SCHEMA_VERSION`, `MAX_CONNECTIONS_BYTES`.
