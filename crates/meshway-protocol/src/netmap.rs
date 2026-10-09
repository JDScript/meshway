//! Node↔Server synchronization: one long-poll `POST /agent-api/v1/sync`.
//!
//! The request names what the Node holds; the Server holds the request until
//! the netmap or visible peer facts change, or until its hold timeout, and
//! answers with a lease for the current netmap. Peer facts never authorize
//! anything: they select among candidates the installed netmap allows.
use crate::object::{wire_object, wire_tagged};
use chrono::{DateTime, Utc};
use meshway_configuration::netmap::{CompiledNetmap, NetmapDelta};
use meshway_model::{
    Digest, NetmapLease, NetmapRef, NodeId, Sequence, StateEpoch, Version,
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
    #[serde(deserialize_with = "meshway_serde::unique_map")]
    pub updated: BTreeMap<NodeId, PeerFacts>,
    /// Peers whose facts are no longer visible or current.
    pub removed: Vec<NodeId>,
} }

wire_object! {
#[derive(Clone, Debug, PartialEq, Eq)]
/// Current netmap lease, netmap changes and peer-fact changes for the Node.
pub struct SyncResponse {
    /// Lease bound to the current netmap.
    pub lease: NetmapLease,
    /// Full, delta or unchanged netmap.
    pub netmap: SyncNetmap,
    /// Peer-fact changes; null when none occurred during the hold.
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
                || facts
                    .base
                    .is_some_and(|base| base.epoch != facts.cursor.epoch))
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
    pub listeners: BTreeMap<String, PeerListenerFacts>,
    /// Exit readiness, or null when the peer has no exit role.
    pub exit: Option<PeerExitFacts> [required = true],
    /// Original report time.
    pub observed_at: DateTime<Utc>,
    /// Time after which the facts are no longer current.
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

    #[test]
    fn unchanged_response_must_match_the_request() {
        let base = reference(3);
        let request = SyncRequest {
            netmap: Some(base),
            state: None,
        };
        let response = SyncResponse {
            lease: NetmapLease::new(base, 0, 60_000).unwrap(),
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
            netmap: SyncNetmap::Unchanged,
            peer_facts: Some(facts(Some(cursor(4)))),
        };
        assert_eq!(response.check(&request), Ok(()));
        response.peer_facts = Some(facts(Some(cursor(2))));
        assert!(response.check(&request).is_err());
        response.peer_facts = Some(facts(None));
        assert_eq!(response.check(&request), Ok(()));
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
