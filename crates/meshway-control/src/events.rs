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
                let changed = updated
                    .get(&self.node)
                    .is_some_and(|node| node.instance_id != self.instance);
                if let Some(node) = updated.get(&self.node) {
                    self.instance = node.instance_id;
                }
                changed || removed.contains(&self.node)
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
            result = self.client.watch_node(self.token.expose(), &options) => result,
        };
        match result {
            Ok(stream) => {
                self.stream = Some(stream);
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
                self.stream.as_mut().expect("opened stream").next().await.map(|result| result.map(|delivery| delivery.data))
            } => result,
        };
        self.ended = result.as_ref().is_none_or(|result| match result {
            Ok(event) => matches!(event.0, StreamEvent::Closed { .. }),
            Err(_) => true,
        });
        if self.ended {
            self.stream = None;
        }
        result.map(|result| EventNotice {
            refresh: result
                .as_ref()
                .is_ok_and(|event| self.interest.changed(event)),
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
            let wrong = match &event.0 {
                StreamEvent::ConfigSnapshot { configuration, .. } => {
                    configuration.network_id != binding.network_id
                        || configuration.node_id != binding.node_id
                }
                StreamEvent::ConfigurationChanged { network_id, .. }
                | StreamEvent::ResourcesChanged { network_id, .. } => {
                    *network_id != binding.network_id
                }
                StreamEvent::EntitiesDelta { updated, .. } => updated
                    .nodes
                    .get(&binding.node_id)
                    .is_some_and(|node| node.network_id != binding.network_id),
                _ => false,
            };
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
            // Initial projections are deliberately discarded: Ready triggers a
            // validated atomic GET/refresh, rather than installing an SSE cache.
            _ => Ok(refresh),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
