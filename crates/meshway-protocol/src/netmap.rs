//! Node↔Server synchronization: one long-poll `POST /agent-api/v1/sync`.
//!
//! The request names what the Node holds; the Server holds the request until
//! the netmap or visible peer facts change, or until its hold timeout, and
//! answers with a lease for the current netmap. Peer facts never authorize
//! anything: they select among candidates the installed netmap allows.
use crate::connectivity::{UdpTraversalRequestIds, UdpTraversalRequests};
use crate::object::{wire_object, wire_tagged};
use chrono::{DateTime, Utc};
use meshway_configuration::{
    execution::ExecutionAddress,
    netmap::{
        AssociationRecordType, CompiledNetmap, DnsAssociationOverlay, NetmapDelta, NodeNetmap,
    },
};
use meshway_model::{
    Digest, KeyId, NetmapLease, NetmapRef, NetworkId, NodeId, NodePublicKey, Sequence, StateEpoch,
    Version,
    exit::{ExitFamily, ExitStatus},
    state::{EndpointCandidate, ListenerPhase, LocalEndpoint, SocketNetwork},
};
use std::collections::BTreeMap;

wire_object! {
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Position in the Server's peer-fact stream; only meaningful within one epoch.
pub struct StateCursor {
    /// State continuity domain; a different epoch requires full facts.
    pub epoch: StateEpoch,
    /// Last applied fact sequence within the epoch.
    pub sequence: Sequence,
} }

wire_object! {
#[derive(Clone, Debug, PartialEq, Eq)]
/// `POST /agent-api/v1/sync`: what the Node currently holds.
pub struct SyncRequest {
    /// Exact verified netmap held; null or omission requests full content.
    pub netmap: Option<NetmapRef>,
    /// Last applied peer-fact cursor; null or omission requests full facts.
    pub state: Option<StateCursor>,
    /// Generation of the Server signing key the Node has pinned; reported for the inventory view only (it selects and authorizes nothing). Null or omission: not reported.
    pub server_key_generation: Option<Version>,
    /// Attempt IDs of the previous answer's `traversalRequests` the Node already holds; a live request not listed here answers the hold at once. Omission: none held.
    #[serde(default)]
    pub traversal_requests: UdpTraversalRequestIds,
    /// Digest of the DNS association overlay the Node holds for `netmap`; null or omission: none.
    #[serde(default)]
    pub associations: Option<Digest>,
    /// The signed lease the Node currently holds, exactly as it verified it. A hold ends at the latest 15 s before this lease ends, and only a lease this Server process issued to the Node bounds a hold at all; null, omission or an unknown lease is answered without holding.
    #[serde(default)]
    pub lease: Option<NetmapLease>,
} }

wire_tagged! {
#[derive(Clone, Debug, PartialEq, Eq)]
/// Netmap part of a sync response.
pub enum SyncNetmap {
    /// Complete netmap; the request held no usable base.
    Full {
        /// Complete content with its digest.
        netmap: Box<CompiledNetmap>
    },
    /// Changes from the netmap named in the request.
    Delta {
        /// Keyed changes, verified by reconstructing the target digest.
        delta: Box<NetmapDelta>
    },
    /// The requested netmap is current; only the lease is renewed.
    Unchanged,
} }

wire_object! {
#[derive(Clone, Debug, PartialEq, Eq)]
/// Peer facts after the request's cursor, scoped by the current netmap.
pub struct PeerFactsDelta {
    /// Cursor the delta applies to; null means `updated` holds every fact the Server knows (for example after a Server restart it may know none yet). It adds and replaces facts only: a Node keeps last known facts until `removed` or its projection drops them.
    pub base: Option<StateCursor> [required = true],
    /// Cursor to send in the next request after applying this delta.
    pub cursor: StateCursor,
    /// Netmap serial whose visible Nodes scoped this projection.
    pub netmap_serial: Version,
    /// Complete replacement facts of each updated peer.
    #[serde(deserialize_with = "meshway_model::wire::unique_map")]
    pub updated: BTreeMap<NodeId, PeerFacts>,
    /// Peers whose facts are no longer visible, were withdrawn by the reporter, or whose Node retired; expiry alone never removes facts.
    pub removed: Vec<NodeId>,
    /// Peers whose facts are unchanged but were reported again: their new freshness bound (`expiresAt` of the facts and of their STUN candidates). Compact on purpose; changed facts appear in `updated` instead.
    #[serde(deserialize_with = "meshway_model::wire::unique_map")]
    pub refreshed: BTreeMap<NodeId, DateTime<Utc>>,
} }

/// Nodes whose facts the owner of `netmap` may see: its projection, without itself.
pub fn fact_visible_peers(netmap: &NodeNetmap) -> std::collections::BTreeSet<NodeId> {
    netmap
        .projection
        .configuration
        .nodes
        .keys()
        .copied()
        .filter(|node| *node != netmap.node_id)
        .collect()
}

impl PeerFacts {
    /// These facts as the owner of `viewer` may see them: none when the
    /// reporter is outside its projection, and listener details only for
    /// listeners it may dial.
    pub fn scoped_for(&self, viewer: &NodeNetmap) -> Option<PeerFacts> {
        if self.node_id == viewer.node_id
            || !viewer
                .projection
                .configuration
                .nodes
                .contains_key(&self.node_id)
        {
            return None;
        }
        let mut facts = self.clone();
        let nodes = &viewer.projection.configuration.nodes;
        facts.connected.retain(|node| nodes.contains_key(node));
        facts.listeners.retain(|key, _| {
            viewer
                .dialers
                .values()
                .any(|dial| dial.peer == self.node_id && dial.listener_key == *key)
        });
        Some(facts)
    }
}

impl PeerFactsDelta {
    /// Apply this delta to a last-known facts ledger: `removed` deletes,
    /// `updated` replaces per peer. A null base deletes nothing: a Server
    /// that just restarted knows no facts yet, which is not their removal.
    /// Expiry deletes nothing either. Returns the changed peers, sorted.
    pub fn apply_to(&self, peers: &mut BTreeMap<NodeId, PeerFacts>) -> Vec<NodeId> {
        let mut changed: Vec<NodeId> = Vec::new();
        for peer in &self.removed {
            if peers.remove(peer).is_some() {
                changed.push(*peer);
            }
        }
        for (peer, until) in &self.refreshed {
            if let Some(facts) = peers.get_mut(peer)
                && *until > facts.expires_at
            {
                facts.expires_at = *until;
                for listener in facts.listeners.values_mut() {
                    for candidate in &mut listener.stun_candidates {
                        candidate.expires_at = candidate.expires_at.max(*until);
                    }
                }
                changed.push(*peer);
            }
        }
        for (peer, facts) in &self.updated {
            if facts.node_id != *peer {
                continue;
            }
            if peers.get(peer) != Some(facts) {
                changed.push(*peer);
                peers.insert(*peer, facts.clone());
            }
        }
        changed.sort();
        changed.dedup();
        changed
    }
}

wire_object! {
#[derive(Clone, Debug, PartialEq, Eq)]
/// Server Ed25519 key that signs netmap leases.
pub struct ServerSigningKey {
    /// Stable identifier of this Server signing key.
    pub key_id: KeyId,
    /// Key generation; a rotated key has a larger generation.
    pub key_generation: Version,
    /// Ed25519 public key.
    pub public_key: NodePublicKey,
} }

wire_object! {
#[derive(Clone, Debug, PartialEq, Eq)]
/// Server signature over `netmap_lease_transcript(network, node, lease, Server, keyId, keyGeneration)`.
pub struct NetmapSignature {
    /// Signing key; must equal the key pinned at enrollment.
    pub key_id: KeyId,
    /// Generation of the signing key.
    pub key_generation: Version,
    /// Unpadded base64url Ed25519 signature (64 bytes, 86 characters); public, not a secret.
    pub signature: String,
} }

/// Signer kind encoded in the netmap lease transcript.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum NetmapSigner {
    /// The Node's own registered control key (local acceptance evidence only).
    Node = 0,
    /// The Server signing key pinned at enrollment (authoritative).
    Server = 1,
}

/// Canonical transcript of one netmap lease for one Node and signer key:
/// domain, network, Node, revision, serial, content digest, lease interval,
/// signer kind, key ID and generation; fixed width, big-endian. The digest
/// covers the content, so full and delta deliveries are both covered once the
/// receiver checks its reconstructed digest against `lease.netmap`.
pub fn netmap_lease_transcript(
    network: NetworkId,
    node: NodeId,
    lease: &NetmapLease,
    signer: NetmapSigner,
    key_id: KeyId,
    key_generation: Version,
) -> Vec<u8> {
    let mut message = Vec::with_capacity(160);
    message.extend_from_slice(b"meshway/netmap-lease/v1\0");
    message.extend_from_slice(network.as_uuid().as_bytes());
    message.extend_from_slice(node.as_uuid().as_bytes());
    message.extend_from_slice(&lease.netmap.revision.get().to_be_bytes());
    message.extend_from_slice(&lease.netmap.serial.get().to_be_bytes());
    message.extend_from_slice(lease.netmap.digest.as_bytes());
    message.extend_from_slice(&lease.issued_at_ms.to_be_bytes());
    message.extend_from_slice(&lease.expires_at_ms.to_be_bytes());
    message.push(signer as u8);
    message.extend_from_slice(key_id.as_uuid().as_bytes());
    message.extend_from_slice(&key_generation.get().to_be_bytes());
    message
}

/// Canonical transcript of one signing-key handover for one network, signed
/// by the endorsing key: domain, network, endorsing key ID and generation,
/// successor key ID, generation and Ed25519 public key, and `notBefore`
/// milliseconds; fixed width, big-endian.
pub fn netmap_key_rotation_transcript(
    network: NetworkId,
    previous_key_id: KeyId,
    previous_key_generation: Version,
    next: &ServerSigningKey,
    not_before_ms: u64,
) -> Vec<u8> {
    let mut message = Vec::with_capacity(160);
    message.extend_from_slice(b"meshway/netmap-key-rotation/v1\0");
    message.extend_from_slice(network.as_uuid().as_bytes());
    message.extend_from_slice(previous_key_id.as_uuid().as_bytes());
    message.extend_from_slice(&previous_key_generation.get().to_be_bytes());
    message.extend_from_slice(next.key_id.as_uuid().as_bytes());
    message.extend_from_slice(&next.key_generation.get().to_be_bytes());
    message.extend_from_slice(next.public_key.bytes.as_bytes());
    message.extend_from_slice(&not_before_ms.to_be_bytes());
    message
}

wire_object! {
#[derive(Clone, Debug, PartialEq, Eq)]
/// Handover from the key that signed `signature` to its successor, present only during the Server's rotation window.
pub struct NetmapKeyRotation {
    /// Successor key; its generation is exactly one above the generation of `signature`.
    pub next: ServerSigningKey,
    /// Earliest time at which a Node may pin `next`.
    #[serde(rename = "notBefore", with = "meshway_model::timestamp")]
    pub not_before_ms: u64 [value_type = meshway_model::timestamp::Timestamp],
    /// Unpadded base64url Ed25519 signature by the key of `signature` over `netmap_key_rotation_transcript(network, signature.keyId, signature.keyGeneration, next, notBefore)`.
    pub endorsement: String,
    /// Signature of the same lease by `next`; a Node that already pinned `next` verifies this one instead of `signature`.
    pub lease_signature: NetmapSignature,
} }

wire_object! {
#[derive(Clone, Debug, PartialEq, Eq)]
/// Current netmap lease, netmap changes and peer-fact changes for the Node.
pub struct SyncResponse {
    /// Lease bound to the current netmap.
    pub lease: NetmapLease,
    /// Server signature over this lease for the requesting Node.
    pub signature: NetmapSignature,
    /// Full, delta or unchanged netmap.
    pub netmap: SyncNetmap,
    /// Peer-fact changes or freshness piggybacked on a normal hold response; a freshness-only response keeps the cursor unchanged. Null when no facts need delivery.
    pub peer_facts: Option<PeerFactsDelta> [required = true],
    /// Signing-key handover while the Server's rotation window is open; null otherwise.
    pub key_rotation: Option<NetmapKeyRotation> [required = true],
    /// Live UDP traversal attempts other Nodes prepared toward this Node that it has not accepted yet; empty when none. Hints only, outside the signed lease.
    pub traversal_requests: UdpTraversalRequests,
    /// The Node's current DNS association overlay when it differs from the one the request named (an empty `associations` list clears it); null when unchanged.
    pub associations: Option<DnsAssociationsUpdate> [required = true],
} }

wire_object! {
#[derive(Clone, Debug, PartialEq, Eq)]
/// A signed DNS association overlay for the requesting Node.
pub struct DnsAssociationsUpdate {
    /// Complete overlay for the netmap of this answer's lease.
    pub overlay: DnsAssociationOverlay,
    /// Signature over `dns_associations_transcript` by the key of the answer's `signature`.
    pub signature: NetmapSignature,
    /// Signature of the same overlay by the successor key while `keyRotation` is present; null otherwise.
    pub rotation_signature: Option<NetmapSignature> [required = true],
} }

/// Canonical transcript of one DNS association overlay for one Node and
/// signer key: domain, network, Node, the overlay's netmap reference, its
/// digest, signer kind, key ID and generation; fixed width, big-endian.
pub fn dns_associations_transcript(
    network: NetworkId,
    node: NodeId,
    netmap: &NetmapRef,
    digest: &Digest,
    key_id: KeyId,
    key_generation: Version,
) -> Vec<u8> {
    let mut message = Vec::with_capacity(176);
    message.extend_from_slice(b"meshway/dns-associations/v1\0");
    message.extend_from_slice(network.as_uuid().as_bytes());
    message.extend_from_slice(node.as_uuid().as_bytes());
    message.extend_from_slice(&netmap.revision.get().to_be_bytes());
    message.extend_from_slice(&netmap.serial.get().to_be_bytes());
    message.extend_from_slice(netmap.digest.as_bytes());
    message.extend_from_slice(digest.as_bytes());
    message.push(NetmapSigner::Server as u8);
    message.extend_from_slice(key_id.as_uuid().as_bytes());
    message.extend_from_slice(&key_generation.get().to_be_bytes());
    message
}

wire_object! {
#[derive(Clone, Debug, PartialEq, Eq)]
/// `POST /agent-api/v1/dns-associations`: an exit resolver's answer to one query of a source it serves, whose name a name rule of that source decides.
pub struct DnsAssociationReport {
    /// Netmap the reporting exit held when it served the query.
    pub netmap: NetmapRef,
    /// Single-use identifier of this completion.
    pub report_id: meshway_model::RequestId,
    /// Source whose authenticated query was answered.
    pub source: NodeId,
    /// Source egress selection identity from the served DNS service.
    pub generation: Digest,
    /// Queried name, ASCII lowercase without a trailing dot.
    pub name: String,
    /// Record type of the query.
    pub record_type: AssociationRecordType,
    /// CNAME chain of the answer in order from `name`; at most 8.
    pub cnames: Vec<String>,
    /// Public addresses of the answer for `recordType`, sorted and unique; 1 to 16.
    pub addresses: Vec<ExecutionAddress>,
    /// Shortest TTL of the chain and address records, in seconds.
    pub ttl_seconds: u32,
    /// When the exit's resolver answered.
    #[serde(rename = "resolvedAt", with = "meshway_model::timestamp")]
    pub resolved_at_ms: u64 [value_type = meshway_model::timestamp::Timestamp],
} }

wire_object! {
#[derive(Clone, Debug, PartialEq, Eq)]
/// An accepted association; the source receives it in its next sync answer.
pub struct DnsAssociationAccepted {
    /// End of the accepted association.
    #[serde(rename = "expiresAt", with = "meshway_model::timestamp")]
    pub expires_at_ms: u64 [value_type = meshway_model::timestamp::Timestamp],
} }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InconsistentSyncResponse;

impl std::fmt::Display for InconsistentSyncResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("sync response is inconsistent with its lease or request")
    }
}

impl std::error::Error for InconsistentSyncResponse {}

impl SyncResponse {
    /// Structural consistency with the request only; the receiver still
    /// verifies digests and its own network/Node binding before saving.
    pub fn check(&self, request: &SyncRequest) -> Result<(), InconsistentSyncResponse> {
        self.lease
            .validate()
            .map_err(|_| InconsistentSyncResponse)?;
        let target = match &self.netmap {
            SyncNetmap::Full { netmap } => netmap.reference(),
            SyncNetmap::Delta { delta } if Some(delta.base) == request.netmap => delta.target,
            SyncNetmap::Unchanged => request.netmap.ok_or(InconsistentSyncResponse)?,
            SyncNetmap::Delta { .. } => return Err(InconsistentSyncResponse),
        };
        if target != self.lease.netmap {
            return Err(InconsistentSyncResponse);
        }
        if let Some(update) = &self.associations
            && (update.overlay.netmap != target
                || update.overlay.validate().is_err()
                || update.signature.key_id != self.signature.key_id
                || update.signature.key_generation != self.signature.key_generation
                || update
                    .rotation_signature
                    .as_ref()
                    .map(|s| (s.key_id, s.key_generation))
                    != self
                        .key_rotation
                        .as_ref()
                        .map(|r| (r.lease_signature.key_id, r.lease_signature.key_generation)))
        {
            return Err(InconsistentSyncResponse);
        }
        // A Node is never asked to accept its own attempt, and each attempt appears once.
        let requests = self.traversal_requests.as_slice();
        if requests.iter().enumerate().any(|(index, request)| {
            requests[..index]
                .iter()
                .any(|other| other.attempt_id == request.attempt_id)
                || (request.carrier.dialer != request.peer_id
                    && request.carrier.listener != request.peer_id)
                || request.carrier.dialer == request.carrier.listener
                || request.carrier.protocol != meshway_model::TransportProtocol::NativeUdp
        }) {
            return Err(InconsistentSyncResponse);
        }
        if let Some(rotation) = &self.key_rotation
            && (Ok(rotation.next.key_generation) != self.signature.key_generation.checked_next()
                || rotation.next.key_id == self.signature.key_id
                || rotation.lease_signature.key_id != rotation.next.key_id
                || rotation.lease_signature.key_generation != rotation.next.key_generation)
        {
            return Err(InconsistentSyncResponse);
        }
        if let Some(facts) = &self.peer_facts
            && (facts.netmap_serial != target.serial
                || facts.base.is_some_and(|base| Some(base) != request.state)
                || facts.base.is_some_and(|base| {
                    base.epoch != facts.cursor.epoch || base.sequence > facts.cursor.sequence
                })
                || facts
                    .removed
                    .iter()
                    .collect::<std::collections::BTreeSet<_>>()
                    .len()
                    != facts.removed.len()
                || facts.removed.iter().any(|node| {
                    facts.updated.contains_key(node) || facts.refreshed.contains_key(node)
                })
                || facts
                    .refreshed
                    .keys()
                    .any(|node| facts.updated.contains_key(node))
                || facts.updated.iter().any(|(node, value)| {
                    *node != value.node_id
                        || value.expires_at <= value.observed_at
                        || value.connected.contains(node)
                        || value.connected.len() > MAX_CONNECTED_FACTS
                        || !value.connected.is_sorted_by(|a, b| a < b)
                }))
        {
            return Err(InconsistentSyncResponse);
        }
        Ok(())
    }
}

wire_object! {
#[derive(Clone, Debug, PartialEq, Eq)]
/// Observed resources of one peer listener.
pub struct PeerListenerFacts {
    /// Listener lifecycle as reported by the peer.
    pub phase: ListenerPhase,
    /// Outer socket protocol.
    pub network: SocketNetwork,
    /// Addresses the peer derived from its own interfaces.
    pub local_endpoints: Vec<LocalEndpoint>,
    /// STUN-observed mappings of this listener socket.
    pub stun_candidates: Vec<EndpointCandidate>,
} }

wire_object! {
#[derive(Clone, Debug, PartialEq, Eq)]
/// Exit readiness of a peer's installed role; selection input only.
pub struct PeerExitFacts {
    /// Role generation from the peer's netmap that was probed.
    pub role_generation: Digest,
    /// Reported lifecycle; unavailable carries its typed cause.
    pub status: ExitStatus,
    /// Families actually installed.
    pub installed_families: Vec<ExitFamily>,
} }

/// Peers whose channels one State report may list (its `links` bound).
pub const MAX_CONNECTED_FACTS: usize = 256;

wire_object! {
#[derive(Clone, Debug, PartialEq, Eq)]
/// Current dynamic facts of one peer, disclosed only to Nodes whose netmap lists it.
pub struct PeerFacts {
    /// Peer described by these facts.
    pub node_id: NodeId,
    /// Listener facts by listener key; endpoint details only reach adjacent Nodes.
    #[serde(deserialize_with = "meshway_model::wire::unique_map")]
    pub listeners: BTreeMap<String, PeerListenerFacts>,
    /// Exit readiness, or null when the peer has no exit role.
    pub exit: Option<PeerExitFacts> [required = true],
    /// Nodes of the viewer's projection the peer reports an authenticated channel with, ascending and unique; it orders relay candidates and authorizes nothing. Omission: not reported (a Server before this field).
    #[serde(default)]
    pub connected: Vec<NodeId>,
    /// Original report time.
    pub observed_at: DateTime<Utc>,
    /// Freshness bound: afterwards the facts are stale (last known) but stay until replaced or removed; STUN candidates and exit readiness are usable only before their own expiry.
    pub expires_at: DateTime<Utc>,
} }

#[cfg(test)]
mod tests {
    use super::*;
    use meshway_model::Revision;

    fn reference(serial: u64) -> NetmapRef {
        NetmapRef {
            revision: Revision::MIN,
            serial: Version::new(serial).unwrap(),
            digest: Digest::from_bytes([serial as u8; 32]),
        }
    }

    fn signature() -> NetmapSignature {
        NetmapSignature {
            key_id: "00000000-0000-4000-8000-0000000000aa".parse().unwrap(),
            key_generation: Version::MIN,
            signature: "A".repeat(86),
        }
    }

    #[test]
    fn lease_transcript_binds_signer_kind_and_key() {
        let lease = NetmapLease::new(reference(3), 1, 2).unwrap();
        let network: NetworkId = "00000000-0000-4000-8000-000000000001".parse().unwrap();
        let node: NodeId = "00000000-0000-4000-8000-000000000002".parse().unwrap();
        let key = signature().key_id;
        let server = netmap_lease_transcript(
            network,
            node,
            &lease,
            NetmapSigner::Server,
            key,
            Version::MIN,
        );
        let local =
            netmap_lease_transcript(network, node, &lease, NetmapSigner::Node, key, Version::MIN);
        assert!(server.starts_with(b"meshway/netmap-lease/v1\0"));
        assert_eq!(server.len(), 24 + 16 + 16 + 8 + 8 + 32 + 8 + 8 + 1 + 16 + 8);
        assert_eq!(server[120], 1);
        assert_eq!(local[120], 0);
        assert_ne!(server, local);
    }

    #[test]
    fn rotation_transcript_binds_network_both_keys_and_not_before() {
        let network: NetworkId = "00000000-0000-4000-8000-000000000001".parse().unwrap();
        let next = ServerSigningKey {
            key_id: "00000000-0000-4000-8000-0000000000bb".parse().unwrap(),
            key_generation: Version::new(2).unwrap(),
            public_key: NodePublicKey {
                algorithm: meshway_model::KeyAlgorithm::Ed25519,
                bytes: Digest::from_bytes([7; 32]),
            },
        };
        let previous = signature().key_id;
        let message = netmap_key_rotation_transcript(network, previous, Version::MIN, &next, 9);
        assert!(message.starts_with(b"meshway/netmap-key-rotation/v1\0"));
        assert_eq!(message.len(), 31 + 16 + 16 + 8 + 16 + 8 + 32 + 8);
        assert_eq!(&message[message.len() - 8..], &9u64.to_be_bytes());
        let other: NetworkId = "00000000-0000-4000-8000-000000000003".parse().unwrap();
        assert_ne!(
            message,
            netmap_key_rotation_transcript(other, previous, Version::MIN, &next, 9)
        );
        assert_ne!(
            message,
            netmap_key_rotation_transcript(network, previous, Version::MIN, &next, 10)
        );
    }

    #[test]
    fn a_rotation_must_name_the_next_generation_and_sign_the_lease_with_it() {
        let base = reference(3);
        let request = SyncRequest {
            netmap: Some(base),
            state: None,
            server_key_generation: Some(Version::MIN),
            traversal_requests: Default::default(),
            associations: None,
            lease: None,
        };
        let next = ServerSigningKey {
            key_id: "00000000-0000-4000-8000-0000000000bb".parse().unwrap(),
            key_generation: Version::new(2).unwrap(),
            public_key: NodePublicKey {
                algorithm: meshway_model::KeyAlgorithm::Ed25519,
                bytes: Digest::from_bytes([7; 32]),
            },
        };
        let rotation = NetmapKeyRotation {
            lease_signature: NetmapSignature {
                key_id: next.key_id,
                key_generation: next.key_generation,
                signature: "B".repeat(86),
            },
            next,
            not_before_ms: 1,
            endorsement: "C".repeat(86),
        };
        let mut response = SyncResponse {
            lease: NetmapLease::new(base, 0, 60_000).unwrap(),
            signature: signature(),
            netmap: SyncNetmap::Unchanged,
            peer_facts: None,
            key_rotation: Some(rotation.clone()),
            traversal_requests: Default::default(),
            associations: None,
        };
        assert_eq!(response.check(&request), Ok(()));
        let text = serde_json::to_string(&response).unwrap();
        assert!(text.contains(r#""keyRotation":{"next":"#) && text.contains(r#""notBefore":"#));
        assert_eq!(
            serde_json::from_str::<SyncResponse>(&text).unwrap(),
            response
        );
        let broken: [fn(&mut NetmapKeyRotation); 4] = [
            |r| r.next.key_generation = Version::new(3).unwrap(),
            |r| r.next.key_id = "00000000-0000-4000-8000-0000000000aa".parse().unwrap(),
            |r| r.lease_signature.key_id = "00000000-0000-4000-8000-0000000000cc".parse().unwrap(),
            |r| r.lease_signature.key_generation = Version::MIN,
        ];
        for (index, breaks) in broken.into_iter().enumerate() {
            let mut bad = rotation.clone();
            breaks(&mut bad);
            response.key_rotation = Some(bad);
            assert!(response.check(&request).is_err(), "case {index}");
        }
    }

    #[test]
    fn unchanged_response_must_match_the_request() {
        let base = reference(3);
        let request = SyncRequest {
            netmap: Some(base),
            state: None,
            server_key_generation: None,
            traversal_requests: Default::default(),
            associations: None,
            lease: None,
        };
        let response = SyncResponse {
            lease: NetmapLease::new(base, 0, 60_000).unwrap(),
            signature: signature(),
            netmap: SyncNetmap::Unchanged,
            peer_facts: None,
            key_rotation: None,
            traversal_requests: Default::default(),
            associations: None,
        };
        assert_eq!(response.check(&request), Ok(()));
        let stale = SyncRequest {
            netmap: Some(reference(2)),
            state: None,
            server_key_generation: None,
            traversal_requests: Default::default(),
            associations: None,
            lease: None,
        };
        assert!(response.check(&stale).is_err());
        let text = serde_json::to_string(&response).unwrap();
        assert!(
            text.contains(r#""netmap":{"type":"unchanged"}"#)
                && text.contains(r#""peerFacts":null"#)
        );
        assert_eq!(
            serde_json::from_str::<SyncResponse>(&text).unwrap(),
            response
        );
        assert!(serde_json::from_str::<SyncRequest>(r#"{"netmap":null,"x":1}"#).is_err());
        assert_eq!(
            serde_json::from_str::<SyncRequest>("{}").unwrap(),
            SyncRequest {
                netmap: None,
                state: None,
                server_key_generation: None,
                traversal_requests: Default::default(),
                associations: None,
                lease: None,
            }
        );
    }

    #[test]
    fn peer_facts_must_continue_the_request_cursor() {
        let base = reference(3);
        let epoch: StateEpoch = "00000000-0000-4000-8000-000000000009".parse().unwrap();
        let cursor = |n| StateCursor {
            epoch,
            sequence: Sequence::new(n).unwrap(),
        };
        let facts = |base| PeerFactsDelta {
            refreshed: BTreeMap::new(),
            base,
            cursor: cursor(5),
            netmap_serial: base_serial(),
            updated: BTreeMap::new(),
            removed: vec![],
        };
        fn base_serial() -> Version {
            Version::new(3).unwrap()
        }
        let request = SyncRequest {
            netmap: Some(base),
            state: Some(cursor(4)),
            server_key_generation: None,
            traversal_requests: Default::default(),
            associations: None,
            lease: None,
        };
        let mut response = SyncResponse {
            lease: NetmapLease::new(base, 0, 60_000).unwrap(),
            signature: signature(),
            netmap: SyncNetmap::Unchanged,
            peer_facts: Some(facts(Some(cursor(4)))),
            key_rotation: None,
            traversal_requests: Default::default(),
            associations: None,
        };
        assert_eq!(response.check(&request), Ok(()));
        response.peer_facts = Some(facts(Some(cursor(2))));
        assert!(response.check(&request).is_err());
        response.peer_facts = Some(facts(None));
        assert_eq!(response.check(&request), Ok(()));
        let node: NodeId = "00000000-0000-4000-8000-000000000001".parse().unwrap();
        let mut invalid = facts(Some(cursor(4)));
        invalid.cursor = cursor(3);
        response.peer_facts = Some(invalid);
        assert!(response.check(&request).is_err());
        let mut invalid = facts(None);
        invalid.removed = vec![node, node];
        response.peer_facts = Some(invalid);
        assert!(response.check(&request).is_err());
        let now = Utc::now();
        let mut invalid = facts(None);
        invalid.removed = vec![node];
        invalid.updated.insert(
            node,
            PeerFacts {
                node_id: node,
                listeners: BTreeMap::new(),
                exit: None,
                connected: Vec::new(),
                observed_at: now,
                expires_at: now + chrono::Duration::seconds(60),
            },
        );
        response.peer_facts = Some(invalid);
        assert!(response.check(&request).is_err());
    }

    #[test]
    fn listener_fact_maps_reject_duplicates_instead_of_last_wins() {
        let text = r#"{"nodeId":"00000000-0000-4000-8000-000000000001","listeners":{"udp":{"phase":"listening","network":"udp","localEndpoints":[],"stunCandidates":[]}},"exit":null,"observedAt":"2026-10-10T00:00:00Z","expiresAt":"2026-10-10T00:01:00Z"}"#;
        let facts: PeerFacts = serde_json::from_str(text).unwrap();
        let entry = serde_json::to_string(&facts.listeners["udp"]).unwrap();
        let duplicated = text.replacen(
            "\"listeners\":{",
            &format!("\"listeners\":{{\"udp\":{entry},"),
            1,
        );
        assert!(serde_json::from_str::<PeerFacts>(&duplicated).is_err());
    }

    #[test]
    fn schemas_are_closed_objects() {
        use utoipa::PartialSchema;
        for schema in [
            SyncRequest::schema(),
            SyncResponse::schema(),
            PeerFactsDelta::schema(),
        ] {
            let text = serde_json::to_string(&schema).unwrap();
            assert!(text.contains("\"additionalProperties\":false"), "{text}");
        }
    }
}
