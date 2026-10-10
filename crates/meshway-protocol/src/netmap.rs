//! Node↔Server synchronization: one long-poll `POST /agent-api/v1/sync`.
//!
//! The request names what the Node holds; the Server holds the request until
//! the netmap or visible peer facts change, or until its hold timeout, and
//! answers with a lease for the current netmap. Peer facts never authorize
//! anything: they select among candidates the installed netmap allows.
use crate::object::{wire_object, wire_tagged};
use chrono::{DateTime, Utc};
use meshway_configuration::netmap::{CompiledNetmap, NetmapDelta, NodeNetmap};
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
    /// Cursor the delta applies to; null means `updated` replaces all facts.
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
    /// Apply this delta to a last-known facts ledger: a null base replaces
    /// it, `removed` deletes, `updated` replaces per peer. Expiry deletes
    /// nothing. Returns the peers whose facts changed, sorted.
    pub fn apply_to(&self, peers: &mut BTreeMap<NodeId, PeerFacts>) -> Vec<NodeId> {
        let mut changed: Vec<NodeId> = Vec::new();
        if self.base.is_none() {
            changed.extend(
                peers
                    .keys()
                    .filter(|peer| !self.updated.contains_key(peer))
                    .copied(),
            );
            peers.retain(|peer, _| self.updated.contains_key(peer));
        }
        for peer in &self.removed {
            if peers.remove(peer).is_some() {
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
                || facts
                    .removed
                    .iter()
                    .any(|node| facts.updated.contains_key(node))
                || facts.updated.iter().any(|(node, value)| {
                    *node != value.node_id || value.expires_at <= value.observed_at
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
    fn unchanged_response_must_match_the_request() {
        let base = reference(3);
        let request = SyncRequest {
            netmap: Some(base),
            state: None,
        };
        let response = SyncResponse {
            lease: NetmapLease::new(base, 0, 60_000).unwrap(),
            signature: signature(),
            netmap: SyncNetmap::Unchanged,
            peer_facts: None,
        };
        assert_eq!(response.check(&request), Ok(()));
        let stale = SyncRequest {
            netmap: Some(reference(2)),
            state: None,
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
                state: None
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
        };
        let mut response = SyncResponse {
            lease: NetmapLease::new(base, 0, 60_000).unwrap(),
            signature: signature(),
            netmap: SyncNetmap::Unchanged,
            peer_facts: Some(facts(Some(cursor(4)))),
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
