# Experimental dataplane integration contract

This crate is a **static private-config lab**, independent of controller HTTP contracts. It provides authenticated native UDP datagrams, native TCP streams, and an authenticated hop over any already-connected duplex stream (including an external VLESS adapter). It is not WireGuard, a TUN implementation, or a production routing/ACL implementation.

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

Hop wire revision 2 requires **fresh transport-key confirmation** after the two Noise IK flights. The responder cannot publish/replace a channel merely after recognizing the replayable first IK message. The initiator sends an encrypted confirmation under the newly derived key; the responder verifies it and returns an encrypted ACK. The initiator waits for that ACK before publishing. Confirmation and subsequent application data share one uninterrupted cipher nonce counter and replay window. The Noise hop prologue is now `meshway/lab/hop/2` plus the carrier discriminator; all experimental peers must be rebuilt together.

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

`statistics()` reports received/sent application envelopes, successfully enqueued forwarding attributed to the ingress transport, and drops (including malformed handshakes/records and saturated queues). Heartbeats are excluded from application counts. UDP channels expire after six seconds without authenticated traffic and use one-second authenticated heartbeats. Stream disconnection is detected by EOF/I/O failure; probe/handshake operations have deadlines. These counters do not represent TUN bytes, delivered IP packets, or throughput measurements.

Secrets and application bytes are redacted from Debug/errors. Configuration serialization intentionally includes the private key for persistence: do not log serialized configuration. Runtime shutdown is explicit and bounded; dropping the last `Runtime` also signals shutdown. See tests for native UDP, TCP/duplex stream and reverse accepted-channel fixtures.

## Checked boundary and remaining limitations

- Native UDP datagrams are capped at `MAX_NATIVE_UDP_DATAGRAM = 9000`; stream records at `MAX_FRAME_SIZE = 16384`. All actual probe/packet payloads are capped at 8192 bytes. Config count/key/path validation, connection semaphore, bounded queues, pending-session caps, handshake/probe timeouts and receive replay windows are enforced.
- Native UDP and real loopback TCP three-node tests cover SHA dialing LAX and NYC traffic returning through that accepted channel; duplex tests additionally force tiny partial reads/writes. Test-only relay observation verifies that even hop-decrypted transit bodies do not contain application plaintext. Captured stream bytes likewise contain no plaintext. These are regression checks alongside Noise, not a formal security proof.
- Tests cover unknown/wrong keys, wrong-carrier and E2E-only-identity hop denial, endpoint rejection of a relay impersonating the source, nonce/ciphertext tampering, hop and endpoint replay, bounded out-of-order UDP, oversized frames/datagrams, unauthorized initiation/wrong predecessor, explicit reverse grants, cached packet sessions/batches, missing mandatory-hop failure, duplicate priority and deterministic cleanup. Security regressions additionally replay captured first IK flights onto new real TCP/UDP connections against live channels and restarted responders, exercise fresh-nonce UDP ACK-loss recovery, and force capacity-one sender rollover while the receiver key remains valid. Old endpoint Init/ciphertext replay, unrelated-origin admission and the exact 3L state bound are tested. Paused-monotonic-clock regressions drop the first E2E application packet in both initial setup and capacity-one rollover, advance five seconds past the three-second hop timeout, and verify cached-key delivery. Delayed/expired Ready and lost-Ready pending-state expiry are also tested.
- This crate does not claim measured per-hop trace, controller integration, packet origin proofs at relays, authorization from routing policy, production key rotation, forward-secure post-compromise ratcheting, PMTU discovery, automatic IP policy, congestion control, kernel batching, zero-copy forwarding, or validated iperf throughput. Actual VLESS wire compatibility and Linux TUN/iperf/Docker integration are the coordinator/adapter boundary.
- Native TCP active channels do not yet send heartbeat probes; a silently black-holed TCP connection can remain selected until the OS reports failure. Native UDP uses authenticated heartbeat expiration. Candidate failover changes only the transport to the same named peer; it never changes mandatory logical hops.
