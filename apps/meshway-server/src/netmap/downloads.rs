//! In-flight budget of rule-set artifact downloads
//! (`GET /agent-api/v1/artifacts/{digest}`): at most [`MAX_DOWNLOADS`] at
//! once, [`PER_NODE`] of one Node, and [`MAX_BYTES`] of artifact bytes held
//! for them together. A download is admitted before its bytes are loaded and
//! keeps its permit until its response body is finished or dropped
//! (cancelled); one that does not fit at once is `Busy` (503 with
//! `Retry-After`), which a Node retries with its own backoff.

use crate::error::Error;
use meshway_model::NodeId;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

pub(crate) const MAX_DOWNLOADS: usize = 16;
/// What a Node itself fetches in parallel.
pub(crate) const PER_NODE: usize = 2;
/// Four of the largest artifacts (`meshway_rules::MAX_ARTIFACT_BYTES`).
pub(crate) const MAX_BYTES: usize = 4 * meshway_rules::MAX_ARTIFACT_BYTES;

pub(crate) struct Downloads {
    count: Arc<Semaphore>,
    bytes: Arc<Semaphore>,
    nodes: Mutex<HashMap<NodeId, usize>>,
    per_node: usize,
    max_bytes: usize,
}

impl Default for Downloads {
    fn default() -> Self {
        Self::new(MAX_DOWNLOADS, PER_NODE, MAX_BYTES)
    }
}

/// One admitted download; dropping it returns its budget.
pub struct Permit {
    _count: OwnedSemaphorePermit,
    _bytes: OwnedSemaphorePermit,
    node: NodeId,
    downloads: Arc<Downloads>,
}

impl Downloads {
    pub(crate) fn new(count: usize, per_node: usize, bytes: usize) -> Self {
        Self {
            count: Arc::new(Semaphore::new(count)),
            bytes: Arc::new(Semaphore::new(bytes)),
            nodes: Mutex::default(),
            per_node,
            max_bytes: bytes,
        }
    }

    /// Admits a download of `len` bytes for `node`, or `Busy`.
    pub(crate) fn admit(self: &Arc<Self>, node: NodeId, len: usize) -> Result<Permit, Error> {
        if len > self.max_bytes {
            return Err(Error::Internal("artifact larger than the download budget"));
        }
        let bytes = u32::try_from(len.max(1)).map_err(|_| Error::Busy)?;
        let mut nodes = self.nodes.lock().unwrap_or_else(|error| error.into_inner());
        let held = nodes.get(&node).copied().unwrap_or(0);
        if held >= self.per_node {
            return Err(Error::Busy);
        }
        let count = Arc::clone(&self.count)
            .try_acquire_owned()
            .map_err(|_| Error::Busy)?;
        let bytes = Arc::clone(&self.bytes)
            .try_acquire_many_owned(bytes)
            .map_err(|_| Error::Busy)?;
        nodes.insert(node, held + 1);
        Ok(Permit {
            _count: count,
            _bytes: bytes,
            node,
            downloads: Arc::clone(self),
        })
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        let mut nodes = self
            .downloads
            .nodes
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(held) = nodes.get_mut(&self.node) {
            *held -= 1;
            if *held == 0 {
                nodes.remove(&self.node);
            }
        }
    }
}

/// The bytes of an admitted download with its permit, held by the response
/// body until it is finished or dropped.
pub struct Download {
    pub bytes: Vec<u8>,
    pub permit: Permit,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(n: u128) -> NodeId {
        NodeId::new(uuid::Uuid::from_u128(n)).unwrap()
    }

    #[test]
    fn downloads_are_bounded_per_node_in_number_and_in_bytes_until_released() {
        let downloads = Arc::new(Downloads::new(3, 2, 100));
        let a1 = downloads.admit(node(1), 10).unwrap();
        let a2 = downloads.admit(node(1), 10).unwrap();
        // A third of the same Node.
        assert!(matches!(downloads.admit(node(1), 10), Err(Error::Busy)));
        // Bytes: 20 of 100 held, 81 do not fit.
        assert!(matches!(downloads.admit(node(2), 81), Err(Error::Busy)));
        let b1 = downloads.admit(node(2), 80).unwrap();
        // Number: three in flight.
        assert!(matches!(downloads.admit(node(3), 0), Err(Error::Busy)));
        drop(a1);
        let a3 = downloads.admit(node(1), 0).unwrap();
        drop((a2, a3, b1));
        assert!(downloads.nodes.lock().unwrap().is_empty());
        assert_eq!(downloads.count.available_permits(), 3);
        assert_eq!(downloads.bytes.available_permits(), 100);
        // Never admitted at all: larger than the whole budget.
        assert!(matches!(
            downloads.admit(node(4), 101),
            Err(Error::Internal(_))
        ));
    }

    #[test]
    fn the_default_budget_admits_the_largest_artifact() {
        let downloads = Arc::new(Downloads::default());
        let held: Vec<_> = (0..4)
            .map(|n| {
                downloads
                    .admit(node(n + 1), meshway_rules::MAX_ARTIFACT_BYTES)
                    .unwrap()
            })
            .collect();
        assert!(matches!(downloads.admit(node(9), 1), Err(Error::Busy)));
        drop(held);
    }
}
