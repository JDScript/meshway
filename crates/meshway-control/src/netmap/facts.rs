//! `peer-facts.json`: the last known peer facts of the held netmap, so that a
//! restart without the Server still has dial addresses.
//!
//! ```json
//! { "format": "meshway-peer-facts-cache/1",
//!   "networkId": …, "nodeId": …, "netmap": NetmapRef,
//!   "cursor": StateCursor,
//!   "peers": { NodeId: PeerFacts } }
//! ```
//!
//! Not signed and not authority: facts only add addresses to try. Admission
//! of every carrier, channel and packet stays with the verified netmap and
//! the authenticated handshake, so a forged or stale address can at most
//! fail to connect. Loading binds the file to this identity and filters it
//! by the loaded netmap's projection; facts outside it are dropped. The
//! cursor only shapes the replay; after a restart Control asks the Server
//! for complete facts.
use crate::{Error, storage};
use meshway_configuration::netmap::NodeNetmap;
use meshway_model::{NetmapRef, NetworkId, NodeId};
use meshway_protocol::{PeerFacts, StateCursor};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Arc};
use zeroize::Zeroizing;

pub(crate) const FILE: &str = "peer-facts.json";
const FORMAT: &str = "meshway-peer-facts-cache/1";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Cache {
    format: String,
    network_id: NetworkId,
    node_id: NodeId,
    netmap: NetmapRef,
    cursor: StateCursor,
    peers: BTreeMap<NodeId, PeerFacts>,
}

/// Last known facts of one netmap, as Control holds and saves them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Known {
    pub netmap: NetmapRef,
    pub cursor: StateCursor,
    /// Shared with the published [`super::KnownPeers`]; copied on write.
    pub peers: Arc<BTreeMap<NodeId, PeerFacts>>,
}

impl Known {
    /// Keep only what the owner of `netmap` may see, under its reference.
    pub fn scope(&mut self, netmap: &NodeNetmap, reference: NetmapRef) {
        self.netmap = reference;
        self.peers = Arc::new(
            self.peers
                .iter()
                .filter_map(|(peer, facts)| Some((*peer, facts.scoped_for(netmap)?)))
                .collect(),
        );
    }

    /// Same addresses and readiness: freshness alone is not worth a write.
    pub fn same_content(&self, other: &Self) -> bool {
        self.netmap == other.netmap
            && self.peers.len() == other.peers.len()
            && self.peers.iter().all(|(peer, facts)| {
                other.peers.get(peer).is_some_and(|known| {
                    known.listeners == facts.listeners && known.exit == facts.exit
                })
            })
    }
}

pub(super) fn encode(
    known: &Known,
    network_id: NetworkId,
    node_id: NodeId,
) -> Result<Zeroizing<Vec<u8>>, Error> {
    storage::encode(&Cache {
        format: FORMAT.to_owned(),
        network_id,
        node_id,
        netmap: known.netmap,
        cursor: known.cursor,
        peers: (*known.peers).clone(),
    })
}

/// The saved facts for this identity, scoped by the loaded `netmap`. A
/// missing, unreadable or foreign file is simply no facts.
pub(super) fn load(
    directory: &meshway_private_storage::Directory,
    netmap: &NodeNetmap,
    reference: NetmapRef,
) -> Option<Known> {
    let bytes = storage::read(directory, FILE).ok()??;
    let cache = storage::decode::<Cache>(&bytes).ok()?;
    if cache.format != FORMAT
        || cache.network_id != netmap.network_id
        || cache.node_id != netmap.node_id
        || cache
            .peers
            .iter()
            .any(|(peer, facts)| facts.node_id != *peer)
    {
        return None;
    }
    let mut known = Known {
        netmap: cache.netmap,
        cursor: cache.cursor,
        peers: Arc::new(cache.peers),
    };
    known.scope(netmap, reference);
    Some(known)
}
