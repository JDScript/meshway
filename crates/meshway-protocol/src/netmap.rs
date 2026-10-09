//! Node API netmap distribution and dynamic peer facts.
//!
//! The query response is the only carrier of netmap content and leases; the
//! `netmap.changed` notice only wakes the Node. Peer facts never authorize
//! anything: they select among candidates the installed netmap already allows.
use crate::object::wire_object;
use chrono::{DateTime, Utc};
use meshway_configuration::netmap::{CompiledNetmap, NetmapDelta};
use meshway_model::{
    Digest, NetmapLease, NetmapRef, NetworkId, NodeId, StateEpoch, Version,
    exit::{ExitFamily, ExitStatus},
    state::{EndpointCandidate, ListenerPhase, LocalEndpoint, SocketNetwork},
};
use std::collections::BTreeMap;

wire_object! {
#[derive(Clone, Debug, PartialEq, Eq)]
/// `POST /agent-api/v1/netmap/query`: fetch the current netmap and a fresh lease.
pub struct NetmapQueryRequest {
    /// Exact netmap the Node holds, verified locally; null or omission requests a full netmap.
    pub base: Option<NetmapRef>,
} }

wire_object! {
#[derive(Clone, Debug, PartialEq, Eq)]
/// Current netmap of the authenticated Node with a lease for exactly that content.
/// Both `full` and `delta` null means the base is current and only the lease is renewed.
pub struct NetmapQueryResponse {
    /// Lease bound to the current netmap.
    pub lease: NetmapLease,
    /// Complete netmap when no usable base was supplied; null otherwise.
    pub full: Option<CompiledNetmap> [required = true],
    /// Changes from the supplied base; null when returning full content or renewing only.
    pub delta: Option<NetmapDelta> [required = true],
} }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InconsistentNetmapResponse;

impl std::fmt::Display for InconsistentNetmapResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("netmap query response is inconsistent with its lease or base")
    }
}

impl std::error::Error for InconsistentNetmapResponse {}

impl NetmapQueryResponse {
    /// Structural consistency only; the receiver still verifies content
    /// digests and its own network/Node binding before saving.
    pub fn check(&self, base: Option<&NetmapRef>) -> Result<(), InconsistentNetmapResponse> {
        self.lease
            .validate()
            .map_err(|_| InconsistentNetmapResponse)?;
        let target = match (&self.full, &self.delta) {
            (Some(full), None) => full.reference(),
            (None, Some(delta)) if Some(&delta.base) == base => delta.target,
            (None, None) => *base.ok_or(InconsistentNetmapResponse)?,
            _ => return Err(InconsistentNetmapResponse),
        };
        if target != self.lease.netmap {
            return Err(InconsistentNetmapResponse);
        }
        Ok(())
    }
}

wire_object! {
#[derive(Clone, Debug, PartialEq, Eq)]
/// `netmap.changed` SSE payload: a newer netmap exists; query to obtain it.
pub struct NetmapChanged {
    /// Network of the changed netmap.
    pub network_id: NetworkId,
    /// Node whose netmap changed.
    pub node_id: NodeId,
    /// New current serial.
    pub serial: Version,
} }

wire_object! {
#[derive(Clone, Debug, PartialEq, Eq)]
/// `netmap.delta` SSE payload: changes from the Node's last acknowledged netmap.
/// A Node holding a different base ignores it and queries instead.
pub struct NetmapDeltaNotice {
    /// Lease bound to the delta target.
    pub lease: NetmapLease,
    /// Changes from an exact base.
    pub delta: NetmapDelta,
} }

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

wire_object! {
#[derive(Clone, Debug, PartialEq, Eq)]
/// `state.delta` SSE payload for a Node: replacement facts of visible peers.
pub struct NodeStateDelta {
    /// State continuity domain of this update.
    pub state_epoch: StateEpoch,
    /// Netmap serial whose peers and edges scoped this projection.
    pub netmap_serial: Version,
    /// Cursor of the preceding applied event.
    pub base_cursor: String,
    /// Resume cursor after applying this delta.
    pub cursor: String,
    /// Complete replacement facts of each updated peer.
    #[serde(deserialize_with = "meshway_serde::unique_map")]
    pub updated: BTreeMap<NodeId, PeerFacts>,
    /// Peers whose facts are no longer visible or current.
    pub removed: Vec<NodeId>,
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
    fn renewal_only_response_must_match_the_base() {
        let base = reference(3);
        let response = NetmapQueryResponse {
            lease: NetmapLease::new(base, 0, 60_000).unwrap(),
            full: None,
            delta: None,
        };
        assert_eq!(response.check(Some(&base)), Ok(()));
        assert!(response.check(Some(&reference(2))).is_err());
        assert!(response.check(None).is_err());
        let text = serde_json::to_string(&response).unwrap();
        assert!(text.contains("\"full\":null") && text.contains("\"delta\":null"));
        assert_eq!(
            serde_json::from_str::<NetmapQueryResponse>(&text).unwrap(),
            response
        );
        assert!(serde_json::from_str::<NetmapQueryRequest>(r#"{"base":null,"x":1}"#).is_err());
        assert_eq!(
            serde_json::from_str::<NetmapQueryRequest>("{}").unwrap(),
            NetmapQueryRequest { base: None }
        );
    }

    #[test]
    fn schemas_are_closed_objects() {
        use utoipa::PartialSchema;
        for schema in [
            NetmapQueryResponse::schema(),
            NodeStateDelta::schema(),
            NetmapDeltaNotice::schema(),
        ] {
            let text = serde_json::to_string(&schema).unwrap();
            assert!(text.contains("\"additionalProperties\":false"), "{text}");
        }
    }
}
