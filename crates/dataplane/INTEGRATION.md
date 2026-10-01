# Experimental dataplane integration contract

## Managed adjacency API (integration names frozen)

This section is the core/agent handoff contract. Adjacency-only managed use configures
`allowed_paths: []`; authenticated adjacency and its control ping grant no endpoint
packet, E2E probe, TUN or transit authorization. The explicit managed IPv4 packet
mode supplies only the intersection of controller-issued full directional grants
and the routing compiler's exact paths. Endpoint-only keys receive no hop carrier
grants. The Agent owns coherent snapshot validation, inner-IP admission, TUN and
fail-closed routes; those are not inferred by this crate. Config/lease changes replace the
runtime via `shutdown()` and `Runtime::start`; `Runtime` is cloneable and also works
inside an `Arc`.

```rust,ignore
ListenerConfig {
    transport: TransportKind::NativeTcp,
    bind: "127.0.0.1:0".parse()?,
    id: Some(listener_id.clone()),
    allowed_peers: Some(vec![peer_node_id.clone()]),
}
OutgoingCandidate {
    peer: peer_node_id.clone(), transport: TransportKind::NativeTcp,
    endpoint: resolved_socket_addr.to_string(), priority: 0,
    id: Some(candidate_id.clone()),
}
// Native listeners/candidates propagate this metadata automatically. An external
// acceptor (e.g. after VLESS setup) supplies locally validated listener admission:
runtime.attach_stream_with_context(stream, StreamRole::Acceptor,
    TransportKind::Vless, 0, StreamContext {
        listener_id: Some(listener_id),
        allowed_peers: Some(vec![peer_node_id.clone()]),
        remote_addr: Some(actual_socket_peer_addr),
        ..Default::default()
    }).await?;
// External dialer: candidate_id and optional actual remote_addr; no listener fields.
// Existing attach_stream / attach_stream_with_priority use StreamContext::default().
let connections: Vec<PeerConnection> = runtime.peer_connections();
// Use the proof-bound result for status; never combine an earlier connections
// snapshot with the RTT of a later probe (the selected candidate can change).
let observed: NeighborProbeObservation = runtime.probe_neighbor_observed(
    &peer_node_id, std::time::Duration::from_secs(2)).await?;
let connection = observed.connection;
let round_trip = observed.round_trip;
```

New exported types:
- `StreamContext { listener_id: Option<String>, candidate_id: Option<String>,
  allowed_peers: Option<Vec<String>>, remote_addr: Option<SocketAddr> }`,
  `Clone + Debug + Default`. Acceptor context disallows candidate IDs; dialer context
  disallows listener IDs and allowlists. Names are nonempty, at most 128 ASCII
  alphanumeric / `-_.:` bytes. Remote address is supplied by the trusted local
  adapter from its socket, never from peer protocol metadata.
- `PeerConnection { peer_node_id: String, transport: TransportKind,
  initiator: bool, listener_id: Option<String>, candidate_id: Option<String>,
  remote_addr: Option<SocketAddr> }`, `Clone + Debug + Serialize`.
  `initiator` is whether this local node dialed. No channel/handshake tokens or
  cryptographic secrets are exposed. Entries appear only after fresh key confirmation;
  they are authenticated channel observations, not a health/delivery promise.
- `NeighborProbeObservation { connection: PeerConnection, round_trip: Duration }`,
  `Clone + Debug + Serialize`, returned by
  `probe_neighbor_observed(&self, peer: &str, timeout: Duration) -> Result<NeighborProbeObservation>`.
  Connection metadata and private channel token are captured together from the
  selected channel when registering the probe. The response must match this exact
  channel, and the same token must still be current at completion. Metadata is
  never re-read from a potentially replaced channel after the response. Replacement
  before the call begins yields the new channel's proof and metadata; replacement
  during the call returns `Unavailable`. This is a **point-in-time observation**,
  not a guarantee that the selected connection will remain current afterward.
  No channel tokens are exposed in public fields, Debug or serialization.
  Existing `probe_neighbor(peer, timeout) -> Result<Duration>` projects this result's
  `round_trip` and remains source-compatible. Managed status reporters should use
  `probe_neighbor_observed` directly rather than joining independent snapshots.
- Native `ListenerConfig` adds serde-default `id: Option<String>` and
  `allowed_peers: Option<Vec<String>>`; `OutgoingCandidate` adds serde-default
  `id: Option<String>`. Existing Rust struct literals must add these fields (`None`
  for legacy lab semantics). `BoundListener` also exposes `id: Option<String>`.

Listener allowlists contain unique known peer IDs with the listener's carrier grant.
`None` retains explicit legacy global carrier grants; `Some([])` denies everyone.
Identity pinning and carrier grants remain mandatory, including external streams.
Admission is checked before the IK response and before publication. Listener A's
allowlist never authorizes listener B. Native outgoing UDP sockets reject incoming
Hello/acceptor handshakes; only the configured dial attempt can establish a channel,
which remains fully bidirectional after confirmation.

Node IDs accept 1–64 ASCII alphanumeric / `-_` bytes, including full controller
64-hex IDs, without aliases or truncation. Longer IDs increase packet overhead;
the existing 1280-byte-inner short-lab-path calculation is unchanged and does not
establish an IP MTU guarantee for controller IDs.

Neighbor ping uses a fixed-size, hop-encrypted control frame and a fresh random
128-bit request ID, matched to the exact current confirmed channel and deadline.
It echoes no application payload, is never forwarded, and never enters packet/E2E
session caches. Calls are bounded by `max_pending_probes` and one outstanding call
per channel; inbound requests have a per-channel budget of eight requests per
one-second window (excess is dropped, without a reply). Timeout must
be nonzero and at most 60 seconds. Timeout invalidates only the channel actually
probed, allowing candidate reconnect; a newer replacement survives. Shutdown or
replacement wakes pending calls. Old `probe(path, payload)` is unchanged.

`Error::Capacity` indicates concurrent-call/global pending or bounded queue
admission failure. `Error::Unavailable` indicates no selected channel or a channel
replaced/closed during the call; `Error::Timeout` removes only the exact probed
channel. Shutdown returns `Error::Closed`. Dropping/cancelling a probe future cleans
up its pending registration without disconnecting. Schedule managed probes below
the responder rate limit (e.g. one per second), and await the core timeout rather
than wrapping the call in a shorter external timeout. Native outgoing candidates
reconnect automatically; the external VLESS adapter owns its reconnect loop.
Neighbor control is excluded from application `received`/`sent` counts, uses the
existing bounded ingress/outgoing queues, and failures retain typed drop attribution.

Hop wire revision **3** adds the distinct neighbor-control frame namespace and uses
`meshway/lab/hop/3` plus the carrier discriminator, with revision-3 confirmation
constants. **Rebuild all peers together**; revision-2 peers cannot authenticate.
E2E packet encoding and short-path datagram overhead remain unchanged.

This crate supplies **static-config experimental dataplane and managed networking primitives**, independent of controller HTTP contracts. It provides authenticated native UDP datagrams, native TCP streams, and an authenticated hop over any already-connected duplex stream (including an external VLESS adapter). The agent maps validated controller configuration onto these primitives; TUN and IP routing/authorization integration belong to the Agent's explicit managed-packet or isolated lab boundary.

## Public API

```rust,ignore
use meshway_dataplane::{Identity, LabConfig, Runtime, StreamRole, TransportKind};

let identity = Identity::generate()?;
// Deliberate secret export; persist using the caller's private atomic-file primitive.
let private_hex = identity.private_key_hex();
let public_hex = identity.public_key_hex();
let config: LabConfig = serde_json::from_str(&private_config_json)?;
config.validate()?;
let runtime = Runtime::start(config).await?;

// VLESS adapter performs VLESS setup outside this crate. The UUID is NOT host auth.
// Both sides subsequently perform pinned-static Noise IK authentication here.
runtime.attach_stream(stream, StreamRole::Dialer { peer: "lax".into() },
                      TransportKind::Vless).await?;
runtime.attach_stream(accepted_stream, StreamRole::Acceptor,
                      TransportKind::Vless).await?;
runtime.wait_for_peer("sha", std::time::Duration::from_secs(5)).await?;
let reply = runtime.probe(vec!["nyc".into(), "lax".into(), "sha".into()],
                          b"private probe".to_vec()).await?;
assert_eq!(reply.payload, b"private probe");
let counters = runtime.statistics();
runtime.shutdown().await;
```

`attach_stream<S: AsyncRead + AsyncWrite + Unpin + Send + 'static>` owns the stream. `BoxedStream` and `DuplexStream` are exported for dynamic factories. Dialer/acceptor describes connection establishment only: an accepted channel supports requests in both business directions. `TransportKind::{NativeUdp, NativeTcp, Vless}` labels counters; UDP cannot be attached as a stream.

`attach_stream` returns after Noise authentication and channel registration. `connected_peers() -> Vec<String>` and `peer_transport(&str) -> Option<TransportKind>` provide synchronous, non-waiting channel snapshots for the CLI's VLESS reconnect loop/status. Stream EOF/I/O failure removes the channel; duplicate replacement cannot remove the newer channel when the older task exits. Snapshots are observations, not delivery guarantees.

Hop wire revision 3 preserves **fresh transport-key confirmation** after the two Noise IK flights. The responder cannot publish/replace a channel merely after recognizing the replayable first IK message. The initiator sends an encrypted confirmation under the newly derived key; the responder verifies it and returns an encrypted ACK. The initiator waits for that ACK before publishing. Confirmation, neighbor control and subsequent application data share one uninterrupted cipher nonce counter and replay window. The Noise hop prologue is `meshway/lab/hop/3` plus the carrier discriminator; all experimental peers must be rebuilt together.

For streams, the existing handshake deadline covers both IK and confirmation. UDP has typed Hello/Response/Data/Confirm/ACK datagrams; response-wait and confirmation-wait each expire within `handshake_timeout_ms`. Unfinished and live hop states share the `max_connections` semaphore and an unfinished state never enters the channel registry. UDP confirms are bound to the source address and full handshake hash. Initiators retry confirmation every 100ms with a **new nonce** until the confirmation deadline. A verified responder can ACK such a fresh confirmation again after ACK loss; exact replayed ciphertext is still rejected and does not refresh liveness. Hello/IK-response loss uses the existing bounded new-handshake reconnect attempt rather than reliable-delivery claims.

Config is strict serde (`deny_unknown_fields`) and contains `node_id`, `private_key` (hex, redacted Debug), `public_key` (hex, checked against private key), `trusted_peers: [{node_id, public_key, transports: ["native_udp"]}]`, `allowed_paths: [["nyc","lax","sha"]]`, `listeners: [{transport: "native_udp"|"native_tcp", bind: "127.0.0.1:0"}]`, `outgoing: [{peer, transport, endpoint, priority}]`, and optional `limits`. Native outgoing candidates are maintained with bounded reconnect attempts; VLESS outgoing establishment belongs to the CLI and uses `attach_stream`. A VLESS listener likewise belongs to the CLI. `bound_listeners()` returns actual socket addresses, including assigned ports. A configured path grants initiation in that **exact order** and control replies on the same hops backward, not reverse initiation or shortcuts/subpaths. To initiate raw packets or probes from SHA, explicitly also configure `["sha","lax","nyc"]` at every node. Every participating node must explicitly allow the full ordered path and trust every named identity's public key.

**Carrier authorization is independent from the identity keyring:** `PeerConfig` now has `pub transports: Vec<TransportKind>`. Its serde default is empty, which means **no hop permitted** (E2E-only identity). Both ends must explicitly permit the actual carrier. For the requested topology LAX's NYC entry has `[NativeUdp]`, LAX's SHA entry has `[Vless]`; NYC/SHA allow their respective carrier for LAX and use `[]` for the remote endpoint. An authenticated SHA key therefore cannot open native UDP to LAX and replace its VLESS channel. Native outgoing configuration is rejected if its carrier is absent from the peer's grants. Stream and UDP handshakes check grants before replying/registration; the channel registry checks again. The carrier kind is also bound into the Noise hop prologue, preventing cross-carrier handshake transplantation. A VLESS label is supplied only by the trusted local adapter after actual VLESS setup.

Priority selects a peer's active channel (lower is preferred); `attach_stream` uses priority 0. `attach_stream_with_priority` is available for CLI-managed candidate ordering. Same-priority duplicates are resolved by the Noise handshake hash, consistently at both ends. The channel initiator's priority is included in the authenticated setup, so both ends agree. There is no per-flow load distribution.

`probe` negotiates a separate pinned-static Noise IK session between the endpoints, with the exact ordered path in the Noise prologue. Relays authenticate their neighbor and validate the claimed full path, direction, and their predecessor/successor before forwarding opaque endpoint ciphertext. Relays do **not** claim cryptographic proof of the original source of every packet; an authorized neighbor can submit authorized path contexts. The final endpoint authenticates the origin key and path before echoing. Probe results contain endpoint-authenticated echo bytes, elapsed duration and the authorized path; the path is not fabricated measured hop observations or a trace.

## Endpoint packet / Linux TUN boundary

```rust,ignore
let mut received = runtime.take_packet_receiver()?; // once per runtime; bounded queue
runtime.send_packet(vec!["nyc".into(), "lax".into(), "sha".into()], raw_ip).await?;
runtime.send_packet_batch(path.clone(), owned_packets).await?;
let packet = received.recv().await.ok_or("runtime closed")?;
// packet.source is authenticated endpoint identity, packet.path is authenticated
// ordered context, packet.payload is owned plaintext only at the final endpoint.
// Coordinator MUST check its static IP-pair/family policy before writing to TUN.
```

`ReceivedPacket { source: String, path: Vec<String>, payload: Vec<u8> }` is public and redacts payload in Debug. Packet mode never automatically echoes data. Probe control is separate and never delivered to the packet receiver. `send_packet`/`send_packet_batch` return after encryption and bounded local enqueue, **not** a remote delivery ACK. Native UDP delivery is best effort, including setup; no retransmission or congestion controller is supplied. Raw-IP parsing, IP-pair ACLs, TUN ownership, routes, NAT and iperf execution belong to the caller.

Packet-mode E2E Noise sessions are cached per ordered path for up to **59 seconds from the sender's Init request start**, including time spent waiting for Ready. Receipt of Ready does not restart this budget, and a Ready arriving after the budget expires is rejected without caching or sending application data. The destination retains the pending key for **60 seconds from processing Init**, so loss of the first encrypted application packet does not strand a still-valid sender cache. Only the first authenticated transport packet promotes the key to active, with a fresh 60-second active retention period. An IK response alone cannot retire the old active key. The receiving endpoint enforces the pinned origin key, exact authorized path and E2E replay window. A destination restart can black-hole an existing cached session until its expiry; there is currently no authenticated session-reset notification. Probe sessions are independently negotiated per call. Batch limits: 16 owned packets, each at most `MAX_PACKET_PAYLOAD = 8192`, and **encoded** carrier frames together at most 65,536 bytes. Exceeding these bounds returns `Oversize`. Queue saturation returns `Capacity`; packet receive saturation is a counted drop.

For `L = limits.max_pending_probes`, the receiver reserves at most **L exact (origin, ordered-path) packet contexts**, each with at most one confirmed active key and one pending replacement. Pending packet keys expire after the cache-compatible 60-second budget; `handshake_timeout_ms` remains a hop-setup bound. A replacement retires the old key only after decrypting a valid packet under the fresh replacement key; replaying an old Init or old ciphertext cannot evict the working key. Different origins/paths cannot borrow this context's rollover slot. Echo setups keep an independent pool of at most L entries and their shorter configured `probe_timeout_ms` expiry. Thus receiving E2E state is explicitly bounded by **2L packet keys + L echo setups = 3L**, including at L=1. Expired entries are purged before admission as well as on cleanup ticks. Source-side outstanding setup registrations and sending-session caches each retain their L limits.

The longer pending retention is an explicit bounded availability tradeoff: an authenticated, path-authorized neighbor replaying a previously valid endpoint Init can occupy that context's single pending slot for up to 60 seconds. It cannot promote a key or retire a working active key without fresh transport-key proof. The experiment does not promise availability against an authorized relay that can also discard transit traffic. Lost Ready remains **best-effort setup loss**: the sender times out and caches no key, while the receiver's pending slot can remain occupied until the 60-second expiry. There is no E2E Ready retransmission/confirmation protocol in this slice. First-application-packet loss is recoverable by a later packet using the same cached key, provided it is sent within the original 59-second sender budget.

For a 1280-byte inner packet on `["nyc","lax","sha"]`, `native_udp_datagram_len(&path, 1280)?` is **1393 bytes** of outer UDP payload: 1421 bytes with an IPv4+UDP header, or 1441 bytes with an IPv6+UDP header. Thus a 1500-byte outer MTU can carry an inner TUN MTU of 1280 (including IPv6 minimum MTU) for this exact lab path. Longer node IDs/more hops increase overhead; use this helper and add 28/48 outer header bytes instead of assuming 1280 always fits. There is no PMTU discovery or Meshway fragmentation/reassembly. The API's 8192-byte maximum does not promise fragmentation-free UDP delivery. Stream carriers are not datagram-MTU constrained.

The public `Batch` carrier type owns bounded frames. `send_packet_batch` uses one queue entry and a cached session without spawning tasks per packet. A relay currently dequeues/forwards individual frames; no zero-copy claim is made. UDP kernel `recvmmsg`/`sendmmsg` acceleration is future work. Stream records are length bounded before allocation. All Noise states use fresh ephemeral handshakes on new sessions, including restarts; replay windows are session-local.

### Cooperative work budget

`limits.work_budget_frames` is an optional integer with **default 16** and validated range **0–64**. `0` disables the explicit frame-budget yields for same-image A/B comparison; Tokio's existing I/O/cooperative scheduling still applies. Older configs that omit this field receive the default, including when other `limits` fields are present. For controlled runs, set it explicitly to `0` or `16` on every participating node while holding `queue_capacity` and worker count fixed.

The task-local counter calls `tokio::task::yield_now().await` after every N completed frame attempts, then resets. Its scope in this first fairness experiment is:

- **UDP socket engine:** one shared counter across received datagrams and outbound application frames. Each received datagram counts once after parsing/authentication/processing and any immediate control reply finish, including rejected input and hop-control datagrams. Every outgoing frame counts after its seal/send attempt, including failure; a 16-frame batch consumes 16 units, not one. Discarding a stale outbound batch likewise accounts for each of its frames. Timer-driven heartbeat/confirmation-retry maintenance keeps its existing scheduling and bounds.
- **Shared dispatcher:** one separate counter per runtime, covering all ingress carriers. Every dequeued envelope counts after synchronous processing/drop accounting completes and all synchronous locks are released, including forwarding, endpoint delivery and rejected envelopes. Its yields let outgoing carrier tasks and endpoint consumers run during continuously ready ingress.

The counter is retained across batches/select iterations and does not allocate, spawn tasks, sleep, or introduce per-frame boxed futures. Yielding is awaited inside the selected handler after complete frame processing, not raced as a selectable branch; it does not cancel partial datagrams, skip fresh-key confirmation, or interrupt crypto/state validation. UDP output remains bounded by the existing 16-frame batch and send deadline; the socket engine returns to its existing shutdown/timer selection between batches. Native TCP/VLESS read/write loops remain on their existing scheduling for this UDP-first A/B slice, while their envelopes do use the budgeted dispatcher.

This is cooperative fairness, not a larger queue or delivery guarantee: real overload can still fill the bounded queues, and `yield_now` does not promise a particular next task or strict scheduling order. Queue capacities, carrier priorities, wire format, authorization, session lifetimes, and crypto bounds are unchanged. Deterministic current-thread regressions use an otherwise idle producer/consumer burst larger than the queue: budget below queue capacity permits consumer progress without queue-full drops, while budget `0` reproduces ingress-full loss. A separate ready-ingress fixture exercises the actual dispatcher and reproduces outgoing-full loss only with its budget disabled. These are scheduling/admission regressions, not throughput measurements.

### Native UDP receive-buffer request

`limits.udp_receive_buffer_bytes` is an independent optional integer with **default 0** and validated range **0–4,194,304 bytes (4 MiB)**. Zero leaves the OS default untouched: no `SO_RCVBUF` set is issued. A nonzero value requests `SO_RCVBUF` through safe `socket2::SockRef` before binding **every native UDP listener and outgoing candidate socket**. For same-image A/B, compare `0` with `1048576` while holding `work_budget_frames`, `queue_capacity`, and worker count fixed.

This is a requested size, not a guaranteed usable payload capacity. The OS may clamp, round, or account overhead differently; Linux commonly reports doubled values. A successful set is accepted without requiring the readback to meet the request. No `SO_RCVBUFFORCE`, privilege bypass, or send-buffer adjustment is used. The kernel's actual per-socket value can be inspected externally (for example, Linux `ss -um` `rb`); no actual-size claim is added to the lab status API.

Native UDP listener and outgoing sockets are all prepared during atomic `Runtime::start`, before spawning tasks. Socket creation, option, nonblocking setup, bind, or Tokio-registration failures return `Error::Io`; an option failure never silently falls back to the OS default. Outgoing hop-handshake/reconnect attempts reuse the prepared, configured socket. This moves outgoing socket setup errors to startup instead of allowing a background bind failure to disappear. The request does not change wire limits, mpsc queue bounds, encryption, replay/session lifetimes, or receive/send batching. Portable socket tests compare readback against a directly configured control socket on the same OS, verify zero preserves defaults and TX is unchanged, and exercise configured sockets through Tokio; they do not assume Linux doubling or establish throughput improvement.

### Native TCP small-record scheduling

`limits.native_tcp_nodelay` is a strict boolean with **default true**. Native TCP
sets `TCP_NODELAY` on both dialed and accepted sockets before Noise authentication.
An explicit `false` reproduces Nagle-enabled behavior for same-image A/B; use
`diagnose.py --first-leg tcp --native-tcp-nodelay off|on`. CLI-owned VLESS sockets
already set NODELAY independently; `attach_stream` cannot configure arbitrary
streams and is unaffected by this option.

An option failure closes the socket before authentication/publication: accepted
connections count an I/O failure and release their permit; dialed connections retry
under the existing bounded reconnect policy. No send/receive buffer sizes, frame
prefixes, flush calls, batch limits, queue bounds or cooperative budgets change.
This option changes kernel packet scheduling, not the byte-stream contract.

Security-impact review of this change: the option is applied **before** the existing
authenticated path. All four Noise/confirmation flights, authenticated priority,
carrier grants, fresh transport-key proof before publication, nonce/replay windows,
path-bound E2E, packet pending/active lifetimes and shutdown selection are unchanged.
The actual socket-option regression covers accepted and dialed TCP; the existing
real-TCP and replay/restart/security tests run with NODELAY enabled. This is a
socket I/O review and regression evidence, not a new cryptographic certification.

`statistics()` reports received/sent application envelopes, successfully enqueued forwarding attributed to the ingress transport, and drops (including malformed handshakes/records and saturated queues). Heartbeats and neighbor-control frames are excluded from application counts. UDP channels expire after six seconds without authenticated traffic and use one-second authenticated heartbeats. Stream disconnection is detected by EOF/I/O failure or caller-scheduled neighbor probe timeout; probe/handshake operations have deadlines. These counters do not represent TUN bytes, delivered IP packets, or throughput measurements.

### Drop attribution and queue observations

`Statistics`, `TransportStatistics`, `DropStatistics`, `QueueStatistics`, `RuntimeQueueStatistics`, and `QueueRejectionStatistics` are typed, serializable lab status snapshots. Existing numeric transport fields `received`, `sent`, `forwarded`, and `dropped` remain. Additional nested fields are:

- `{native_udp,native_tcp,vless}.drop_reasons`: `ingress_queue_full`, `ingress_queue_closed`, `outbound_queue_full`, `outbound_queue_closed`, `endpoint_delivery_full`, `endpoint_delivery_closed`, `authentication`, `replay`, `oversize`, `protocol`, `unavailable`, `unauthorized_path`, `unauthorized_carrier`, `capacity_other`, `stale_channel`, `io`, `timeout`, `closed`, `other`.
- `{native_udp,native_tcp,vless}.outbound_queue_rejections.{full,closed}`: a **secondary attribution view** for the actual failing outgoing carrier. Do not add these to `dropped` or `drop_reasons`. For UDP ingress forwarded onto a full VLESS queue, exactly one drop appears in `native_udp.drop_reasons.outbound_queue_full`, while `vless.outbound_queue_rejections.full` identifies the blocked outgoing carrier. `vless.dropped` is not incremented by that relay failure.
- `queues.ingress.{capacity,occupancy,high_water}` and `queues.endpoint_delivery.{capacity,occupancy,high_water}` describe the runtime-wide shared queues.
- `{native_udp,native_tcp,vless}.outbound_queue.{capacity,occupancy,high_water}` describes the **maximum** occupied entries in any currently selected queue of that carrier, not summed occupancy. UDP channels sharing one socket queue therefore do not multiply its occupancy. The high-water retains the largest sample across that carrier's queues, including replaced/disconnected ones.

Each counted failed attempt selects exactly one reason. `dropped` is computed from the same `drop_reasons` snapshot, so it equals `DropStatistics::total()` even while traffic continues. Queue-specific reasons take precedence over generic `Capacity`/`Unavailable`/`Closed`; `capacity_other` covers other resource admission failures, including bounded endpoint/control-setup state. Unknown session/key or failed authentication is `authentication`; a known superseded channel is `stale_channel`; unauthorized carrier grants are `unauthorized_carrier`. An unrecognized token alone is not sufficient evidence to label traffic stale.

| Owning attempt boundary | Counting unit and transport attribution |
| --- | --- |
| Hop-authenticated ingress enqueue | One envelope per `try_send`; application envelopes increment ingress `received` even if full/closed, then at most one ingress drop. Neighbor control shares bounded queue/drop accounting but is excluded from application `received`. A rejected envelope never reaches dispatch. |
| Dispatcher processing | One dequeued envelope; decode, authorization, E2E authentication/replay, forwarding/control-reply enqueue, or endpoint-delivery failure contributes one ingress drop. Successful forwarding increments ingress `forwarded`. |
| Locally originated outgoing enqueue | One queue entry per failed `try_send`, attributed to the chosen outgoing carrier. A batch is **one admission attempt**, even if it contains multiple frames. Both single-frame setup/probe sends and cached packet batches use this boundary. |
| UDP engine | One rejected received datagram, failed attempted datagram send (including heartbeat), or dequeued outbound batch whose session has disappeared, attributed to native UDP. |
| Stream engine / native listener | One error terminating an established read/write engine; or one rejected native accepted connection due to admission/authentication failure, attributed to its carrier. EOF/I/O is an `io` event, not an estimate of lost queued frames. |

This retains the previously counted aggregate boundaries and additionally exposes formerly uncounted local outgoing queue rejections and discarded stale UDP batches. Pre-enqueue local API validation/setup errors still return their existing `Error`; if no outgoing queue was attempted, they are not assigned an invented ingress/carrier drop. External stream-attachment and outgoing reconnect/setup failures that are returned or retried outside these boundaries are likewise not a complete connection-attempt metric. Shutdown discards/unattempted remainder of a failed stream batch are not retrospectively counted as delivered or dropped frames. These are **stage-attempt counters**, not unique-packet loss accounting, and control/connection errors mean `received - sent` is not a loss estimate.

Queue units are entries, not bytes: ingress/delivery use one envelope/packet per entry; outgoing may hold up to 16 frames per entry. `capacity` is the configured `limits.queue_capacity` (default 64; allowed range remains 1–256), including when no carrier is selected. `occupancy` samples occupied channel permits (including any in-flight reservation). High-water sampling occurs after successful enqueue, on full rejection (which records exact capacity), and at status snapshot; a concurrent consumer can make it miss a transient peak. A successful enqueue establishes a minimum high-water of one. Samples are not an atomic cross-queue view and do not include work already dequeued/in flight. Watermarks use relaxed atomic loads and `fetch_max` only for a new observed peak. Reason counting uses fixed typed atomics with no new per-packet allocations, locks, string labels, timers, or syscalls.

Kernel/socket UDP receive loss happens before these userspace queues and is **not included**. Collect namespace-local `/proc/net/snmp` UDP deltas separately; this instrumentation does not introduce ancillary `SO_RXQ_OVFL` reception. The deterministic queue regression covers shared ingress saturation, cross-carrier outbound full/closed attribution, stale/closed ingress, and watermark retention. The encrypted endpoint regression distinguishes delivery full/closed from bad AEAD and replay, and verifies valid ciphertext still works after a forgery.

Secrets and application bytes are redacted from Debug/errors. Configuration serialization intentionally includes the private key for persistence: do not log serialized configuration. Runtime shutdown is explicit and bounded; dropping the last `Runtime` also signals shutdown. See tests for native UDP, TCP/duplex stream and reverse accepted-channel fixtures.

## Checked boundary and remaining limitations

- Native UDP datagrams are capped at `MAX_NATIVE_UDP_DATAGRAM = 9000`; stream records at `MAX_FRAME_SIZE = 16384`. All actual probe/packet payloads are capped at 8192 bytes. Config count/key/path validation, connection semaphore, bounded queues, pending-session caps, handshake/probe timeouts and receive replay windows are enforced.
- Native UDP and real loopback TCP three-node tests cover SHA dialing LAX and NYC traffic returning through that accepted channel; duplex tests additionally force tiny partial reads/writes. Test-only relay observation verifies that even hop-decrypted transit bodies do not contain application plaintext. Captured stream bytes likewise contain no plaintext. These are regression checks alongside Noise, not a formal security proof.
- Tests cover unknown/wrong keys, wrong-carrier and E2E-only-identity hop denial, endpoint rejection of a relay impersonating the source, nonce/ciphertext tampering, hop and endpoint replay, bounded out-of-order UDP, oversized frames/datagrams, unauthorized initiation/wrong predecessor, explicit reverse grants, cached packet sessions/batches, missing mandatory-hop failure, duplicate priority and deterministic cleanup. Security regressions additionally replay captured first IK flights onto new real TCP/UDP connections against live channels and restarted responders, exercise fresh-nonce UDP ACK-loss recovery, and force capacity-one sender rollover while the receiver key remains valid. Old endpoint Init/ciphertext replay, unrelated-origin admission and the exact 3L state bound are tested. Paused-monotonic-clock regressions drop the first E2E application packet in both initial setup and capacity-one rollover, advance five seconds past the three-second hop timeout, and verify cached-key delivery. Delayed/expired Ready and lost-Ready pending-state expiry are also tested.
- Managed-adjacency regressions in `src/runtime/tests/managed.rs` use real native TCP/UDP sockets with full 64-byte IDs, reject listener-B handshakes before an IK reply despite listener-A authorization, verify dial-only UDP and E2E-only identity denial, and exercise VLESS external stream context. Bidirectional neighbor ping succeeds with empty paths while application/E2E APIs and malicious neighbor Packet injection are denied. Tests cover wrong request IDs, unsolicited/late/replayed replies, exact-frame size, request-rate and pending-call bounds, cancellation cleanup, blackhole timeout/reconnect, replacement-token protection and shutdown of owned streams/tasks. VLESS setup and Docker orchestration remain adapter/coordinator integration tests.
- This crate does not claim measured per-hop trace, controller integration, packet origin proofs at relays, authorization from routing policy, production key rotation, forward-secure post-compromise ratcheting, PMTU discovery, automatic IP policy, congestion control, kernel batching, zero-copy forwarding, or validated iperf throughput. Actual VLESS wire compatibility and Linux TUN/iperf/Docker integration are the coordinator/adapter boundary.
- Native TCP/VLESS callers schedule `probe_neighbor` for active liveness checks; core does not start an implicit periodic stream probe task. Without these calls, a silently black-holed stream can remain selected until the OS reports failure. Native UDP also retains authenticated heartbeat expiration. Candidate failover changes only the transport to the same named peer; it never changes mandatory logical hops.
