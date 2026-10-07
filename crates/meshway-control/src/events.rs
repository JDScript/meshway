//! One protected, typed watchNode connection. The host owns its task and retry.
use crate::{Control, Error};
use meshway_control_client::{Client, ClientError, EventStream, StreamOptions};
use meshway_model::{ConfigurationRef, InstanceId, NetworkId, NodeId, Revision};
use meshway_protocol::{ErrorResponse, NodeEvent, StreamEvent, WireSecret, events::ClosedReason};
use std::{collections::BTreeMap, sync::Arc};
use tokio_util::sync::CancellationToken;

/// A notice retains its authentication scope even while queued at the host.
pub struct EventNotice {
    owner: Arc<()>,
    scope: CancellationToken,
    result: Result<NodeEvent, ClientError<ErrorResponse>>,
    refresh: bool,
}

impl EventNotice {
    pub fn received_event(&self) -> bool {
        self.result.is_ok()
    }
    /// Static event name for credential-free host diagnostics.
    pub fn kind(&self) -> &'static str {
        match &self.result {
            Err(_) => "transport_error",
            Ok(NodeEvent(StreamEvent::Reset { .. })) => "sync.reset",
            Ok(NodeEvent(StreamEvent::ConfigSnapshot { .. })) => "config.snapshot",
            Ok(NodeEvent(StreamEvent::Ready { .. })) => "sync.ready",
            Ok(NodeEvent(StreamEvent::Closed { reason })) => match reason {
                ClosedReason::SessionExpired => "session_expired",
                ClosedReason::MembershipRevoked => "membership_revoked",
                ClosedReason::NodeLeaseExpired => "node_lease_expired",
                ClosedReason::ServerShutdown => "server_shutdown",
            },
            Ok(NodeEvent(StreamEvent::EntitiesDelta { .. })) => "entities.delta",
            Ok(NodeEvent(StreamEvent::StateDelta { .. })) => "state.delta",
            Ok(NodeEvent(StreamEvent::ConfigurationChanged { .. })) => "configuration.changed",
            _ => "projection",
        }
    }
    /// Coalescible invalidation hint, never an execution grant. The host may
    /// retain one pending hint while discarding additional hints until refresh.
    pub fn refresh_requested(&self) -> bool {
        self.refresh
    }

    /// Terminal deliveries must not be dropped when a refresh hint is queued.
    pub fn is_terminal(&self) -> bool {
        matches!(
            &self.result,
            Err(_) | Ok(NodeEvent(StreamEvent::Closed { .. }))
        )
    }
}

/// Single connection; `next` has no retry loop. Drop cancels its protected HTTP
/// request. No cursor is retained because event projections are not our cache.
pub struct NodeEvents {
    owner: Arc<()>,
    client: Client,
    token: WireSecret,
    scope: CancellationToken,
    stream: Option<EventStream<NodeEvent, ErrorResponse>>,
    ended: bool,
    interest: Interest,
    initial_deadline: Option<tokio::time::Instant>,
    initial_refresh: bool,
}

// Replay disclosure includes peer publications and observations. Those changes
// reset the public SSE projection without changing THIS Node's execution target.
// Track hints only; none of these values enter Control's saved/lease authority.
struct Interest {
    network: NetworkId,
    node: NodeId,
    target: Option<ConfigurationRef>,
    revision: Option<Revision>,
    keys: meshway_protocol::NodeKeyMetadata,
    peer_keys: BTreeMap<NodeId, meshway_protocol::NodeKeyMetadata>,
    instance: Option<InstanceId>,
}

impl Interest {
    fn changed(&mut self, event: &NodeEvent) -> bool {
        match &event.0 {
            StreamEvent::EntitiesSnapshot { entities, .. } => {
                let keys = entities
                    .nodes
                    .iter()
                    .map(|(id, node)| (*id, node.keys.clone()))
                    .collect();
                let changed = self.peer_keys != keys;
                self.peer_keys = keys;
                changed
            }
            StreamEvent::ConfigSnapshot { configuration, .. } => {
                let changed = configuration.network_id != self.network
                    || configuration.node_id != self.node
                    || configuration.target != self.target
                    || self.revision.is_none_or(|old| configuration.revision > old);
                self.target = configuration.target.clone();
                self.revision = Some(configuration.revision);
                changed
            }
            StreamEvent::EntitiesDelta {
                updated, removed, ..
            } => {
                let changed = updated.nodes.get(&self.node).is_some_and(|node| {
                    node.network_id != self.network
                        || node.keys != self.keys
                        || node.desired_execution != self.target
                });
                if let Some(node) = updated.nodes.get(&self.node) {
                    self.keys = node.keys.clone();
                    self.target = node.desired_execution.clone();
                }
                let mut keys_changed = false;
                for (id, node) in &updated.nodes {
                    keys_changed |= self.peer_keys.get(id) != Some(&node.keys);
                    self.peer_keys.insert(*id, node.keys.clone());
                }
                for id in &removed.nodes {
                    keys_changed |= self.peer_keys.remove(id).is_some();
                }
                changed || keys_changed || removed.nodes.contains(&self.node)
            }
            StreamEvent::StateDelta {
                updated, removed, ..
            } => {
                let changed = self
                    .instance_changed(updated.get(&self.node).and_then(|node| node.instance_id));
                changed || removed.contains(&self.node)
            }
            StreamEvent::StateSnapshot { state, .. } => {
                let instance = state
                    .nodes
                    .get(&self.node)
                    .and_then(|node| node.instance_id);
                self.instance_changed(instance)
            }
            StreamEvent::ConfigurationChanged {
                network_id,
                revision,
                ..
            } => {
                let changed =
                    *network_id != self.network || self.revision.is_none_or(|old| *revision > old);
                self.revision = Some(*revision);
                changed
            }
            StreamEvent::ResourcesChanged { .. } => true,
            StreamEvent::Reset { reason } => {
                !matches!(reason, meshway_protocol::events::ResetReason::ViewChanged)
            }
            // ViewChanged is followed by a complete projection; compare its own
            // target above instead of refreshing for every peer/sample reset.
            _ => false,
        }
    }

    fn instance_changed(&mut self, instance: Option<InstanceId>) -> bool {
        // A projection without a report has no observed instance. It is not
        // evidence that Control's independently acquired instance disappeared.
        let Some(instance) = instance else {
            return false;
        };
        let changed = self.instance != Some(instance);
        self.instance = Some(instance);
        changed
    }
}

impl NodeEvents {
    /// Open exactly once using the existing protected pool. HTTP acceptance is
    /// separate from receiving the first validated event. Cancellation is EOF.
    pub async fn connect(&mut self) -> Result<bool, Box<EventNotice>> {
        if self.ended || self.scope.is_cancelled() {
            return Ok(false);
        }
        if self.stream.is_some() {
            return Ok(true);
        }
        let options = StreamOptions::default();
        let result = tokio::select! { biased;
            _ = self.scope.cancelled() => return Ok(false),
            result = tokio::time::timeout(std::time::Duration::from_secs(20), self.client.watch_node(self.token.expose(), &options)) => result.unwrap_or(Err(ClientError::Network { status: None })),
        };
        match result {
            Ok(stream) => {
                self.stream = Some(stream);
                self.initial_deadline =
                    Some(tokio::time::Instant::now() + std::time::Duration::from_secs(20));
                Ok(true)
            }
            Err(error) => {
                self.ended = true;
                Err(Box::new(EventNotice {
                    owner: self.owner.clone(),
                    scope: self.scope.clone(),
                    result: Err(error),
                    refresh: false,
                }))
            }
        }
    }

    pub async fn next(&mut self) -> Option<EventNotice> {
        if self.ended || self.scope.is_cancelled() {
            self.stream = None;
            return None;
        }
        match self.connect().await {
            Ok(true) => {}
            Ok(false) => return None,
            Err(notice) => return Some(*notice),
        }
        let result = tokio::select! { biased;
            _ = self.scope.cancelled() => None,
            result = async {
                let next = self.stream.as_mut().expect("opened stream").next();
                let delivery = if let Some(deadline) = self.initial_deadline {
                    tokio::time::timeout_at(deadline, next).await.unwrap_or(Some(Err(ClientError::Network { status: Some(200) })))
                } else {
                    next.await
                };
                delivery.map(|result| result.map(|delivery| delivery.data))
            } => result,
        };
        self.ended = result.as_ref().is_none_or(|result| match result {
            Ok(event) => matches!(event.0, StreamEvent::Closed { .. }),
            Err(_) => true,
        });
        if self.ended {
            self.stream = None;
        }
        // Binding failures must reach Control immediately even before Ready;
        // batching only coalesces ordinary invalidation hints.
        let wrong_binding = result.as_ref().is_some_and(|result| {
            result
                .as_ref()
                .is_ok_and(|event| wrong_binding(event, self.interest.network, self.interest.node))
        });
        let mut refresh = result.as_ref().is_some_and(|result| {
            result
                .as_ref()
                .is_ok_and(|event| self.interest.changed(event))
        });
        if self.initial_deadline.is_some() {
            self.initial_refresh |= refresh;
            refresh = wrong_binding;
            if matches!(&result, Some(Ok(NodeEvent(StreamEvent::Ready { .. })))) {
                self.initial_deadline = None;
                refresh = self.initial_refresh;
            }
        }
        result.map(|result| EventNotice {
            refresh: refresh || wrong_binding,
            owner: self.owner.clone(),
            scope: self.scope.clone(),
            result,
        })
    }
}

impl Control {
    pub(super) fn reset_events(&mut self) {
        self.events_cancel.cancel();
        self.events_cancel = CancellationToken::new();
    }

    /// Uses precisely the same generated client and host-protected socket pool
    /// as every other control operation. The host may move this into one task.
    pub fn watch_node(&self) -> Result<NodeEvents, Error> {
        let instance = self.current_instance()?;
        let binding = self.record.binding.as_ref().ok_or(Error::Lifecycle)?;
        let mut peer_keys = self
            .cached_target()
            .into_iter()
            .flat_map(|target| target.configuration.peers.iter())
            .map(|(id, peer)| {
                (
                    *id,
                    meshway_protocol::NodeKeyMetadata {
                        control: peer.control_key.clone(),
                        dataplane: peer.dataplane_key.clone(),
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        peer_keys.insert(binding.node_id, binding.keys.clone());
        Ok(NodeEvents {
            interest: Interest {
                network: binding.network_id,
                node: binding.node_id,
                target: self
                    .instance
                    .as_ref()
                    .and_then(|instance| instance.lease.target.clone()),
                revision: self
                    .cached_target()
                    .map(|target| target.configuration.revision),
                keys: binding.keys.clone(),
                peer_keys,
                instance: Some(instance.instance_id),
            },
            owner: self.application_owner.clone(),
            client: self.client.clone(),
            token: WireSecret::new(self.token()?.to_owned()),
            scope: self.events_cancel.clone(),
            stream: None,
            ended: false,
            initial_deadline: None,
            initial_refresh: false,
        })
    }

    /// True requests the existing refresh cycle. Events never install a target,
    /// acknowledge a cursor, mint authority, or extend any deadline.
    pub fn receive_event(&mut self, notice: EventNotice) -> Result<bool, Error> {
        if !Arc::ptr_eq(&notice.owner, &self.application_owner) || notice.scope.is_cancelled() {
            return Ok(false);
        }
        let refresh = notice.refresh_requested();
        let event = match notice.result {
            Ok(event) => event,
            Err(error) => {
                // Only an actual HTTP authentication rejection fences authority.
                // EOF, malformed events and delivery failure are watch failures,
                // not evidence that an independently granted lease was revoked.
                if matches!(
                    &error,
                    ClientError::Http {
                        status: 401 | 403 | 404 | 409,
                        ..
                    } | ClientError::Network {
                        status: Some(401 | 403 | 404 | 409)
                    }
                ) {
                    return self.registered::<bool>(Err(error));
                }
                return Ok(false);
            }
        };
        self.ready()?;
        if let Some(binding) = &self.record.binding {
            let wrong = wrong_binding(&event, binding.network_id, binding.node_id);
            if wrong {
                self.fence();
                return Err(Error::Contract);
            }
        }
        match event.0 {
            StreamEvent::Closed { reason } => match reason {
                ClosedReason::SessionExpired | ClosedReason::NodeLeaseExpired => {
                    self.fence();
                    self.session = None;
                    self.authentication = None;
                    self.challenge_intent = None;
                    Err(Error::Expired)
                }
                ClosedReason::MembershipRevoked => {
                    self.fence();
                    self.session = None;
                    self.authentication = None;
                    self.challenge_intent = None;
                    Err(Error::MembershipRevoked)
                }
                ClosedReason::ServerShutdown => Ok(false),
            },
            // Only changed initial projections request a validated atomic
            // refresh at Ready; accepting a connection is not a reset.
            _ => Ok(refresh),
        }
    }
}

fn wrong_binding(event: &NodeEvent, network: NetworkId, node: NodeId) -> bool {
    match &event.0 {
        StreamEvent::ConfigSnapshot { configuration, .. } => {
            configuration.network_id != network || configuration.node_id != node
        }
        StreamEvent::ConfigurationChanged { network_id, .. }
        | StreamEvent::ResourcesChanged { network_id, .. } => *network_id != network,
        StreamEvent::EntitiesSnapshot { entities, .. } => entities
            .nodes
            .get(&node)
            .is_some_and(|entry| entry.network_id != network),
        StreamEvent::EntitiesDelta { updated, .. } => updated
            .nodes
            .get(&node)
            .is_some_and(|entry| entry.network_id != network),
        StreamEvent::StateSnapshot { state, .. } => state.network_id != network,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn interest() -> Interest {
        let network = crate::random_id().unwrap().try_into().unwrap();
        let node = crate::random_id().unwrap().try_into().unwrap();
        let peer = crate::random_id().unwrap().try_into().unwrap();
        let material = crate::identity::Keys::generate().unwrap();
        let keys = meshway_protocol::NodeKeyMetadata {
            control: meshway_model::BoundNodeKey {
                id: crate::random_id().unwrap().try_into().unwrap(),
                generation: meshway_model::Version::MIN,
                public_key: material.control().unwrap(),
            },
            dataplane: meshway_model::BoundNodeKey {
                id: crate::random_id().unwrap().try_into().unwrap(),
                generation: meshway_model::Version::MIN,
                public_key: material.dataplane().unwrap(),
            },
        };
        Interest {
            network,
            node,
            target: None,
            revision: Some(Revision::MIN),
            keys: keys.clone(),
            peer_keys: BTreeMap::from([(peer, keys.clone())]),
            instance: None,
        }
    }

    #[tokio::test]
    async fn heartbeats_cannot_extend_an_incomplete_initial_batch() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            let mut length = 0;
            while !request[..length].ends_with(b"\r\n\r\n") {
                let read = socket.read(&mut request[length..]).await.unwrap();
                assert!(read > 0 && length + read < request.len());
                length += read;
            }
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n").await.unwrap();
            loop {
                if socket.write_all(b": keepalive\n\n").await.is_err() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        });
        let mut stream = NodeEvents {
            owner: Arc::new(()),
            client: Client::new(&origin, reqwest::Client::builder().no_proxy(), 1024).unwrap(),
            token: WireSecret::new("test-token".into()),
            scope: CancellationToken::new(),
            stream: None,
            ended: false,
            interest: interest(),
            initial_deadline: None,
            initial_refresh: false,
        };
        assert!(stream.connect().await.unwrap_or(false));
        stream.initial_deadline =
            Some(tokio::time::Instant::now() + std::time::Duration::from_millis(100));
        let notice = stream.next().await.unwrap();
        assert!(notice.is_terminal());
        assert!(!notice.refresh_requested());
        assert!(stream.next().await.is_none());
        task.abort();
        let _ = task.await;
    }

    #[test]
    fn peer_publications_and_observation_projections_do_not_request_apply() {
        let mut interest = interest();
        let node = interest.node;
        let peer = *interest.peer_keys.keys().next().unwrap();
        let network = interest.network;
        let keys = interest.keys.clone();
        let mut inventory = meshway_protocol::NodeInventoryView {
            id: peer,
            network_id: network,
            registered_at_ms: 0,
            hostname: "peer".into(),
            enabled: true,
            identity: meshway_configuration::NodeIdentity::Tag {
                tags: vec!["peer".into()],
            },
            addresses: meshway_configuration::NodeAddresses {
                ipv4: None,
                ipv6: None,
            },
            keys,
            control_key_revoked_at_ms: None,
            dataplane_key_revoked_at_ms: None,
            can_delete: false,
            desired_execution: None,
            can_revoke_control_key: false,
            can_revoke_dataplane_key: false,
            control_key_created_at_ms: 0,
            dataplane_key_created_at_ms: 0,
        };
        let delta = |inventory: meshway_protocol::NodeInventoryView| {
            NodeEvent(StreamEvent::EntitiesDelta {
                base_cursor: "before".into(),
                cursor: "after".into(),
                updated: meshway_protocol::live::NodeEntitiesView {
                    nodes: BTreeMap::from([(inventory.id, inventory)]),
                },
                removed: meshway_protocol::events::EntityRemovals {
                    nodes: vec![],
                    members: vec![],
                },
            })
        };
        assert!(!interest.changed(&delta(inventory.clone())));
        inventory.desired_execution = Some(serde_json::from_value(serde_json::json!({
            "revision": 2, "executionPublication": crate::random_id().unwrap().to_string(), "digest": "00".repeat(32)
        })).unwrap());
        assert!(!interest.changed(&delta(inventory.clone())));
        inventory.keys.dataplane.generation =
            inventory.keys.dataplane.generation.checked_next().unwrap();
        assert!(interest.changed(&delta(inventory.clone())));
        assert!(!interest.changed(&delta(inventory.clone())));
        inventory.id = node;
        assert!(interest.changed(&delta(inventory)));
        assert!(
            !interest.changed(&NodeEvent(StreamEvent::ObservationRemoved {
                subscription_id: crate::random_id().unwrap().try_into().unwrap(),
            }))
        );
        for (removed, expected) in [(vec![peer], false), (vec![node], true)] {
            assert_eq!(
                interest.changed(&NodeEvent(StreamEvent::StateDelta {
                    state_epoch: crate::random_id().unwrap().try_into().unwrap(),
                    as_of: None,
                    sequence: None,
                    configuration_revision: None,
                    base_cursor: "before".into(),
                    cursor: "after".into(),
                    updated: BTreeMap::new(),
                    removed,
                    links: vec![],
                    paths: vec![],
                })),
                expected
            );
        }
        for (reason, expected) in [
            (meshway_protocol::events::ResetReason::ViewChanged, false),
            (meshway_protocol::events::ResetReason::EpochChanged, true),
            (meshway_protocol::events::ResetReason::Backpressure, true),
        ] {
            assert_eq!(
                interest.changed(&NodeEvent(StreamEvent::Reset { reason })),
                expected
            );
        }
    }

    #[test]
    fn closure_reasons_and_transport_failure_have_distinct_authority_effects() {
        for (reason, expected) in [
            (ClosedReason::SessionExpired, "expired"),
            (ClosedReason::NodeLeaseExpired, "expired"),
            (ClosedReason::MembershipRevoked, "revoked"),
            (ClosedReason::ServerShutdown, "reconnect"),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let mut control = Control::open(
                &directory.path().canonicalize().unwrap().join("node"),
                "http://127.0.0.1:9",
                reqwest::Client::builder().no_proxy(),
            )
            .unwrap();
            let scope = control.events_cancel.clone();
            assert!(
                !control
                    .receive_event(EventNotice {
                        owner: Arc::new(()),
                        refresh: false,
                        scope: scope.clone(),
                        result: Ok(NodeEvent(StreamEvent::Closed {
                            reason: ClosedReason::MembershipRevoked
                        })),
                    })
                    .unwrap()
            );
            assert!(!scope.is_cancelled());
            let result = control.receive_event(EventNotice {
                owner: control.application_owner.clone(),
                refresh: false,
                scope: scope.clone(),
                result: Ok(NodeEvent(StreamEvent::Closed { reason })),
            });
            match expected {
                "expired" => assert!(matches!(result, Err(Error::Expired))),
                "revoked" => assert!(matches!(result, Err(Error::MembershipRevoked))),
                _ => assert!(matches!(result, Ok(false))),
            }
            assert_eq!(scope.is_cancelled(), expected != "reconnect");
            for error in [
                ClientError::Network { status: None },
                ClientError::Decode { status: 200 },
                ClientError::BodyLimit { status: 200 },
                ClientError::Http {
                    status: 503,
                    problem: None,
                },
            ] {
                let scope = control.events_cancel.clone();
                assert!(
                    !control
                        .receive_event(EventNotice {
                            owner: control.application_owner.clone(),
                            refresh: false,
                            scope: scope.clone(),
                            result: Err(error)
                        })
                        .unwrap()
                );
                assert!(!scope.is_cancelled());
            }
        }
    }
}
