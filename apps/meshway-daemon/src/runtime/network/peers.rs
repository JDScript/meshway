//! Peer facts from Control's `sync` long poll: selection input only. The
//! complete current facts are kept here so that a Manager started later (or
//! replaced) receives them at once, and exit readiness is derived for the
//! packet owner's exit selection. Nothing here admits a packet or carrier.
use meshway_connectivity::FactsSink;
use meshway_control::{FactsRejected, PeerFactsSink};
use meshway_model::{NetmapRef, NodeId, exit::ExitFamily, exit::ExitStatus};
use meshway_protocol::{PeerFacts, PeerFactsDelta, StateCursor};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
};

#[derive(Default)]
struct State {
    peers: BTreeMap<NodeId, PeerFacts>,
    cursor: Option<(StateCursor, meshway_model::Version)>,
    manager: Option<FactsSink>,
}

/// Shared between the Control task (sink) and the network worker (Manager
/// attachment and exit readiness).
#[derive(Clone, Default)]
pub(in crate::runtime) struct Peers(Arc<Mutex<State>>);

impl Peers {
    /// The sink handed to `meshway_control::sync_loop`.
    pub fn sink(&self) -> Sink {
        Sink(self.clone())
    }

    /// Attach the current Manager's facts store and replay every known fact
    /// to it as one complete replacement.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub fn attach(&self, mut sink: FactsSink, netmap: NetmapRef) {
        let Ok(mut state) = self.0.lock() else {
            return;
        };
        if let Some((cursor, serial)) = state.cursor {
            let replay = PeerFactsDelta {
                base: None,
                cursor,
                netmap_serial: serial,
                updated: state.peers.clone(),
                removed: Vec::new(),
            };
            if sink.apply(netmap, replay).is_err() {
                return;
            }
        }
        state.manager = Some(sink);
    }

    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub fn detach(&self) {
        if let Ok(mut state) = self.0.lock() {
            state.manager = None;
        }
    }

    /// Exits whose readiness fact is current, ready and has IPv4 installed.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub fn ready_exits(&self) -> BTreeSet<NodeId> {
        let now = chrono::Utc::now();
        self.0.lock().map_or_else(
            |_| BTreeSet::new(),
            |state| {
                state
                    .peers
                    .iter()
                    .filter(|(_, facts)| facts.expires_at > now)
                    .filter(|(_, facts)| {
                        facts.exit.as_ref().is_some_and(|exit| {
                            exit.status == ExitStatus::Ready
                                && exit.installed_families.contains(&ExitFamily::Ipv4)
                        })
                    })
                    .map(|(peer, _)| *peer)
                    .collect()
            },
        )
    }
}

/// [`PeerFactsSink`] for Control. Bounded by the netmap's visible peers.
pub(in crate::runtime) struct Sink(Peers);

impl PeerFactsSink for Sink {
    fn apply(&mut self, netmap: NetmapRef, delta: PeerFactsDelta) -> Result<(), FactsRejected> {
        let mut state = self.0.0.lock().map_err(|_| FactsRejected)?;
        if delta.base.is_none() {
            state.peers.clear();
        }
        for peer in &delta.removed {
            state.peers.remove(peer);
        }
        for (peer, facts) in &delta.updated {
            if facts.node_id == *peer {
                state.peers.insert(*peer, facts.clone());
            }
        }
        state.cursor = Some((delta.cursor, delta.netmap_serial));
        if let Some(manager) = &mut state.manager
            && manager.apply(netmap, delta).is_err()
        {
            // The Manager's bounded queue is full: request complete facts.
            state.cursor = None;
            return Err(FactsRejected);
        }
        Ok(())
    }
}
