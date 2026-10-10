//! One reading of the desired content and the clock, and the one lifetime
//! value shared by circuits, flows, queued routes and outputs.
//!
//! No lease is read here. The owner applies empty content when the lease ends
//! and, before returning, cancels every circuit, flow and queued output; a
//! [`Cap`] carries only an object's own bound (idle, handshake or probe).
use meshway_configuration::{
    NodeAddresses,
    execution::TrustedNode,
    netmap::{CompiledNetmap, NodeNetmap},
};
use meshway_engine::Desired;
use meshway_model::{BoundNodeKey, KeyAlgorithm, NetworkId, NodeId};
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

/// Desired content and one clock reading. Borrowed for one synchronous
/// section; never stored or carried across an await.
#[derive(Clone, Copy)]
pub(crate) struct View<'a> {
    pub desired: &'a Desired,
    pub now_ms: u64,
    pub instant: Instant,
}

impl<'a> View<'a> {
    pub fn of(desired: &'a Desired) -> Self {
        Self {
            desired,
            now_ms: now_ms(),
            instant: Instant::now(),
        }
    }

    pub fn netmap(&self) -> &'a CompiledNetmap {
        self.desired.content()
    }

    pub fn node(&self) -> NodeId {
        self.netmap().netmap.node_id
    }

    pub fn network(&self) -> NetworkId {
        self.netmap().netmap.network_id
    }

    pub fn self_keys(&self) -> &'a TrustedNode {
        &self.netmap().netmap.self_.self_keys
    }

    /// Current Ed25519 control and X25519 data-plane bindings of a path Node.
    pub fn keys(&self, node: NodeId) -> Option<(&'a BoundNodeKey, &'a BoundNodeKey)> {
        let trusted = if node == self.node() {
            self.self_keys()
        } else {
            self.netmap().netmap.projection.inventory.get(&node)?
        };
        valid(&trusted.control_key, &trusted.dataplane_key)
    }

    pub fn now(&self) -> (Instant, u64) {
        (self.instant, self.now_ms)
    }
}

/// Wall-clock milliseconds for INIT windows and peer-supplied UTC bounds.
pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| u64::try_from(elapsed.as_millis()).ok())
        .unwrap_or(0)
}

fn valid<'a>(
    control: &'a BoundNodeKey,
    dataplane: &'a BoundNodeKey,
) -> Option<(&'a BoundNodeKey, &'a BoundNodeKey)> {
    (control.public_key.algorithm == KeyAlgorithm::Ed25519
        && dataplane.public_key.algorithm == KeyAlgorithm::X25519)
        .then_some((control, dataplane))
}

/// Mesh addresses of a Node in a netmap's projection.
pub(crate) fn netmap_addresses(netmap: &NodeNetmap, node: NodeId) -> Option<&NodeAddresses> {
    if node == netmap.node_id {
        Some(&netmap.self_.addresses)
    } else {
        netmap
            .projection
            .configuration
            .nodes
            .get(&node)
            .map(|peer| &peer.addresses)
    }
}

/// A monotonic and an absolute UTC cutoff, both exclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Cap {
    pub at: Instant,
    pub ms: u64,
}

impl Cap {
    pub fn after(instant: Instant, now_ms: u64, duration: Duration) -> Option<Self> {
        Some(Self {
            at: instant.checked_add(duration)?,
            ms: now_ms.checked_add(u64::try_from(duration.as_millis()).ok()?)?,
        })
    }

    /// The monotonic instant equivalent to an absolute UTC expiry, judged at one view.
    pub fn until_ms(instant: Instant, now_ms: u64, ms: u64) -> Option<Self> {
        Some(Self {
            at: instant.checked_add(Duration::from_millis(ms.saturating_sub(now_ms)))?,
            ms,
        })
    }

    pub fn min(self, other: Self) -> Self {
        Self {
            at: self.at.min(other.at),
            ms: self.ms.min(other.ms),
        }
    }

    pub fn live_at(&self, instant: Instant, now_ms: u64) -> bool {
        instant < self.at && now_ms < self.ms
    }
}

/// Restrict an optional cap; `None` means bounded only by the snapshot.
pub(crate) fn restrict(cap: Option<Cap>, other: Option<Cap>) -> Option<Cap> {
    match (cap, other) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// Own cap plus independent cancellation (circuit and, for outputs of one
/// flow, that flow). Its owner cancels it when content no longer admits it.
#[derive(Clone, Default)]
pub(crate) struct Lifetime {
    pub cap: Option<Cap>,
    pub cancellation: CancellationToken,
    pub flow: Option<CancellationToken>,
}

impl Lifetime {
    pub fn new(cap: Option<Cap>, cancellation: CancellationToken) -> Self {
        Self {
            cap,
            cancellation,
            flow: None,
        }
    }

    pub fn restrict(&mut self, cap: Option<Cap>) {
        self.cap = restrict(self.cap, cap);
    }

    pub fn cancelled(&self) -> bool {
        self.cancellation.is_cancelled() || self.flow.as_ref().is_some_and(|t| t.is_cancelled())
    }

    pub fn live_at(&self, instant: Instant, now_ms: u64) -> bool {
        !self.cancelled() && self.cap.is_none_or(|cap| cap.live_at(instant, now_ms))
    }

    /// Wake at cancellation or the own cap.
    pub async fn ended(&self) {
        tokio::select! { biased;
            _ = self.cancellation.cancelled() => {},
            _ = async {
                match &self.flow {
                    Some(token) => token.cancelled().await,
                    None => std::future::pending().await,
                }
            } => {},
            _ = async {
                match self.cap {
                    Some(cap) => tokio::time::sleep_until(cap.at.into()).await,
                    None => std::future::pending().await,
                }
            } => {},
        }
    }
}
