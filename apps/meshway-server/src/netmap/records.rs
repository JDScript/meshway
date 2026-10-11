//! Process-local observations of each Node's sync, for the inventory view
//! only. They authorize nothing and do not survive a Server restart.
use meshway_model::{NetmapLease, NetworkId, NodeId, Version};
use std::collections::BTreeMap;
use std::sync::{LazyLock, Mutex};

/// At most this many Nodes are remembered; the oldest observation goes first.
const MAX_RECORDS: usize = 65_536;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Record {
    /// When the latest sync request of the Node arrived.
    pub(crate) synced_at_ms: Option<u64>,
    /// The latest lease signed for the Node.
    pub(crate) lease: Option<NetmapLease>,
    /// The Server key generation the latest sync request reported pinned.
    pub(crate) pinned_key: Option<Version>,
}

#[derive(Default)]
pub(crate) struct Records {
    entries: Mutex<BTreeMap<(NetworkId, NodeId), Record>>,
}

impl Records {
    pub(crate) fn shared() -> &'static Self {
        static SHARED: LazyLock<Records> = LazyLock::new(Records::default);
        &SHARED
    }

    fn update(&self, network: NetworkId, node: NodeId, edit: impl FnOnce(&mut Record)) {
        let Ok(mut entries) = self.entries.lock() else {
            return;
        };
        if entries.len() >= MAX_RECORDS
            && !entries.contains_key(&(network, node))
            && let Some(oldest) = entries
                .iter()
                .min_by_key(|(_, record)| record.synced_at_ms)
                .map(|(key, _)| *key)
        {
            entries.remove(&oldest);
        }
        edit(entries.entry((network, node)).or_default());
    }

    pub(crate) fn received(
        &self,
        network: NetworkId,
        node: NodeId,
        now_ms: u64,
        pinned_key: Option<Version>,
    ) {
        self.update(network, node, |record| {
            record.synced_at_ms = Some(now_ms);
            record.pinned_key = pinned_key;
        });
    }

    pub(crate) fn issued(&self, network: NetworkId, node: NodeId, lease: NetmapLease) {
        self.update(network, node, |record| record.lease = Some(lease));
    }

    pub(crate) fn get(&self, network: NetworkId, node: NodeId) -> Record {
        self.entries
            .lock()
            .ok()
            .and_then(|entries| entries.get(&(network, node)).copied())
            .unwrap_or_default()
    }
}
