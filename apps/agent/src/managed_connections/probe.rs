//! Read-only handles to the manager's current runtime. No new runtime or grant.
use std::{
    collections::BTreeMap,
    sync::{Arc, Weak},
    time::Duration,
};

use meshway_dataplane::Runtime;
use meshway_protocol::{NodeConfiguration, NodeConnections};
use tokio::{
    sync::watch,
    time::{self, Instant},
};

use super::{Manager, Observation, Phase, session};
use crate::diagnostics::{ProbeError, ProbeObservation, ProbeRequest};

struct Generation {
    runtime: Weak<Runtime>,
    intent: NodeConnections,
    deadline: watch::Receiver<Option<Instant>>,
    observed: watch::Receiver<Observation>,
}

impl Generation {
    fn current(&self) -> bool {
        self.deadline.has_changed().is_ok()
            && self.observed.has_changed().is_ok()
            && self
                .deadline
                .borrow()
                .is_some_and(|deadline| deadline > Instant::now())
            && self.observed.borrow().state == Phase::Applied
    }

    async fn invalidated(&self) {
        let mut deadlines = self.deadline.clone();
        let mut observed = self.observed.clone();
        loop {
            let deadline = *deadlines.borrow_and_update();
            let applied = observed.borrow_and_update().state == Phase::Applied;
            let Some(deadline) = deadline.filter(|_| applied) else {
                return;
            };
            tokio::select! {
                biased;
                _ = time::sleep_until(deadline) => return,
                changed = deadlines.changed() => if changed.is_err() { return; },
                changed = observed.changed() => if changed.is_err() { return; },
            }
        }
    }
}

pub(super) struct Publisher {
    current: watch::Sender<Option<Arc<Generation>>>,
    inventory: watch::Sender<BTreeMap<String, String>>,
}

impl Publisher {
    pub fn new() -> Self {
        Self {
            current: watch::channel(None).0,
            inventory: watch::channel(BTreeMap::new()).0,
        }
    }

    pub fn withdraw(&self) {
        self.current.send_replace(None);
    }

    pub fn publish(
        &self,
        runtime: &Arc<Runtime>,
        intent: &NodeConnections,
        deadline: &watch::Sender<Option<Instant>>,
        observed: &watch::Receiver<Observation>,
    ) {
        self.current.send_replace(Some(Arc::new(Generation {
            runtime: Arc::downgrade(runtime),
            intent: intent.clone(),
            deadline: deadline.subscribe(),
            observed: observed.clone(),
        })));
    }
}

impl Manager {
    /// Captures no strong runtime reference; request lifetimes are lease-fenced.
    pub fn prober(&self) -> Prober {
        Prober {
            current: self.probes.current.subscribe(),
            inventory: self.probes.inventory.subscribe(),
        }
    }

    pub fn set_probe_inventory(&self, configuration: &NodeConfiguration) {
        self.probes.inventory.send_replace(
            std::iter::once(&configuration.node)
                .chain(&configuration.peers)
                .map(|node| (node.id.clone(), node.name.clone()))
                .collect(),
        );
    }
}

#[derive(Clone)]
pub(crate) struct Prober {
    current: watch::Receiver<Option<Arc<Generation>>>,
    inventory: watch::Receiver<BTreeMap<String, String>>,
}

impl Prober {
    fn current(&self, generation: &Arc<Generation>) -> bool {
        self.current.has_changed().is_ok()
            && self
                .current
                .borrow()
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, generation))
            && generation.current()
    }

    fn resolve(
        &self,
        peer: &str,
        intent: &NodeConnections,
    ) -> Result<(String, Option<String>), ProbeError> {
        let inventory = self.inventory.borrow();
        let id = if meshway_protocol::local::valid_credential(peer) {
            peer.to_owned()
        } else {
            let mut matches = inventory.iter().filter(|(_, name)| name.as_str() == peer);
            let (id, _) = matches.next().ok_or(ProbeError::UnconfiguredPeer)?;
            if matches.next().is_some() {
                return Err(ProbeError::AmbiguousPeer);
            }
            id.clone()
        };
        if id == intent.node_id
            || !intent.peers.iter().any(|p| p.node_id == id)
            || !(intent.outgoing.iter().any(|p| p.peer_node_id == id)
                || intent.incoming.iter().any(|p| p.peer_node_id == id))
        {
            return Err(ProbeError::UnconfiguredPeer);
        }
        let name = inventory.get(&id).cloned();
        Ok((id, name))
    }

    /// Probes only the manager's existing authenticated direct neighbor channel.
    /// Both success and failure are fenced before/after await against withdrawal,
    /// lease expiry and semantic/material generation replacement.
    pub async fn probe_neighbor_observed(
        &self,
        request: ProbeRequest,
    ) -> Result<ProbeObservation, ProbeError> {
        request.validate()?;
        let mut current = self.current.clone();
        let generation = current
            .borrow_and_update()
            .clone()
            .ok_or(ProbeError::Unavailable)?;
        if !self.current(&generation) {
            return Err(ProbeError::Unavailable);
        }
        let (peer, name) = self.resolve(&request.peer, &generation.intent)?;
        let runtime = generation
            .runtime
            .upgrade()
            .ok_or(ProbeError::Unavailable)?;
        let deadline = Instant::now() + Duration::from_millis(request.timeout_ms);
        let probe = async {
            // A background probe can temporarily occupy the single channel slot.
            // At most 20 short retries; the total request deadline also bounds it.
            for attempt in 0..=20 {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err(ProbeError::Timeout);
                }
                match runtime.probe_neighbor_observed(&peer, remaining).await {
                    Ok(proof) => {
                        let described = session::describe(&generation.intent, &proof.connection)
                            .ok_or(ProbeError::Unavailable)?;
                        return Ok(ProbeObservation {
                            peer_node_id: described.peer_node_id,
                            peer_name: name,
                            local_node_id: generation.intent.node_id.clone(),
                            initiator_node_id: described.initiator_node_id,
                            transport: described.transport,
                            // Preserve sub-microsecond precision without formatting secrets.
                            rtt_ns: proof.round_trip.as_nanos().min(u64::MAX as u128) as u64,
                        });
                    }
                    Err(meshway_dataplane::Error::Capacity) if attempt < 20 => {
                        time::sleep_until(deadline.min(Instant::now() + Duration::from_millis(25)))
                            .await;
                    }
                    Err(meshway_dataplane::Error::Capacity) => return Err(ProbeError::Busy),
                    Err(meshway_dataplane::Error::Timeout) => return Err(ProbeError::Timeout),
                    Err(meshway_dataplane::Error::Authentication) => {
                        return Err(ProbeError::AuthenticationFailed);
                    }
                    Err(_) => return Err(ProbeError::ChannelUnavailable),
                }
            }
            Err(ProbeError::Busy)
        };
        let result = tokio::select! {
            biased;
            _ = generation.invalidated() => Err(ProbeError::Unavailable),
            _ = current.changed() => Err(ProbeError::Unavailable),
            result = probe => result,
        };
        if !self.current(&generation) {
            return Err(ProbeError::Unavailable);
        }
        result
    }
}
