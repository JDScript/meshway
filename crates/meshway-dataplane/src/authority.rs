//! One guard evaluation of either authority kind, and the one lifetime value
//! shared by circuits, flows, queued routes and output guards.
//!
//! A netmap snapshot is bound by its lease alone. Its deadline lives in the
//! snapshot's shared lease cell and is raised in place by a same-content
//! renewal, so derived state never copies it: a [`Cap`] only carries this
//! object's own shorter bound (idle, handshake, probe or legacy permit), and
//! every check additionally takes a fresh [`View`]. The legacy per-publication
//! target (removed with the daemon switch) adds its permit expiries as caps.
use meshway_authority::{ExecutionSnapshot, Reading};
use meshway_configuration::{
    NodeAddresses,
    execution::{NodeConfiguration, TrustedNode},
    netmap::{CompiledNetmap, NodeNetmap},
};
use meshway_model::{BoundNodeKey, KeyAlgorithm, NetmapLease, NetworkId, NodeId};
use meshway_policy::admission::netmap::Leased;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

/// The configuration a reading authorizes.
#[derive(Clone, Copy)]
pub(crate) enum Auth<'a> {
    /// Legacy per-publication target; removed with the daemon switch (N1h).
    Legacy(&'a NodeConfiguration),
    Netmap {
        netmap: &'a CompiledNetmap,
        lease: NetmapLease,
    },
}

/// Values of one complete snapshot guard evaluation. Borrowed for one
/// synchronous section; never stored or carried across an await.
#[derive(Clone, Copy)]
pub(crate) struct View<'a> {
    pub auth: Auth<'a>,
    pub now_ms: u64,
    pub instant: Instant,
    /// Current snapshot deadline; reporting only, never copied into a cap of
    /// netmap state (it can rise in place).
    pub expires_at: Instant,
    pub expires_at_ms: u64,
}

impl<'a> From<Reading<'a>> for View<'a> {
    fn from(reading: Reading<'a>) -> Self {
        Self {
            auth: Auth::Legacy(reading.configuration),
            now_ms: reading.now_ms,
            instant: reading.instant,
            expires_at: reading.expires_at,
            expires_at_ms: reading.expires_at_ms,
        }
    }
}

impl<'a> View<'a> {
    pub fn of(snapshot: &'a ExecutionSnapshot) -> Result<Self, meshway_authority::Error> {
        if snapshot.is_netmap() {
            let reading = snapshot.netmap_reading()?;
            Ok(Self {
                auth: Auth::Netmap {
                    netmap: reading.netmap,
                    lease: reading.lease,
                },
                now_ms: reading.now_ms,
                instant: reading.instant,
                expires_at: reading.expires_at,
                expires_at_ms: reading.expires_at_ms,
            })
        } else {
            snapshot.reading().map(Self::from)
        }
    }

    pub fn legacy(&self) -> Option<&'a NodeConfiguration> {
        match self.auth {
            Auth::Legacy(cfg) => Some(cfg),
            Auth::Netmap { .. } => None,
        }
    }

    pub fn netmap(&self) -> Option<&'a CompiledNetmap> {
        match self.auth {
            Auth::Netmap { netmap, .. } => Some(netmap),
            Auth::Legacy(_) => None,
        }
    }

    /// Input of `meshway_policy::admission::netmap`, with `now_ms`.
    pub fn leased(&self) -> Option<Leased<'_>> {
        match &self.auth {
            Auth::Netmap { netmap, lease } => Some(Leased {
                netmap: &netmap.netmap,
                lease,
            }),
            Auth::Legacy(_) => None,
        }
    }

    pub fn node(&self) -> NodeId {
        match self.auth {
            Auth::Legacy(cfg) => cfg.node_id,
            Auth::Netmap { netmap, .. } => netmap.netmap.node_id,
        }
    }

    pub fn network(&self) -> NetworkId {
        match self.auth {
            Auth::Legacy(cfg) => cfg.network_id,
            Auth::Netmap { netmap, .. } => netmap.netmap.network_id,
        }
    }

    pub fn self_keys(&self) -> &'a TrustedNode {
        match self.auth {
            Auth::Legacy(cfg) => &cfg.self_keys,
            Auth::Netmap { netmap, .. } => &netmap.netmap.self_.self_keys,
        }
    }

    /// Current Ed25519 control and X25519 data-plane bindings of a path Node.
    pub fn keys(&self, node: NodeId) -> Option<(&'a BoundNodeKey, &'a BoundNodeKey)> {
        let trusted = if node == self.node() {
            self.self_keys()
        } else {
            match self.auth {
                Auth::Legacy(cfg) => {
                    let peer = cfg.peers.get(&node)?;
                    return valid(&peer.control_key, &peer.dataplane_key);
                }
                Auth::Netmap { netmap, .. } => netmap.netmap.projection.inventory.get(&node)?,
            }
        };
        valid(&trusted.control_key, &trusted.dataplane_key)
    }

    pub fn now(&self) -> (Instant, u64) {
        (self.instant, self.now_ms)
    }
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
/// flow, that flow). It never extends the snapshot it is checked against.
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

    /// Effective expiry for reporting, against the snapshot's current deadline.
    pub fn expiry(&self, snapshot: &ExecutionSnapshot) -> Cap {
        let current = Cap {
            at: snapshot.expires_at(),
            ms: snapshot.expires_at_ms(),
        };
        self.cap.map_or(current, |cap| cap.min(current))
    }

    /// Wake at cancellation, the own cap or the snapshot's end (which keeps
    /// following same-content renewals of a netmap lease).
    pub async fn ended(&self, snapshot: &ExecutionSnapshot) {
        let cap_ms = self.cap.map_or(u64::MAX, |cap| cap.ms);
        tokio::select! { biased;
            _ = self.cancellation.cancelled() => {},
            _ = async {
                match &self.flow {
                    Some(token) => token.cancelled().await,
                    None => std::future::pending().await,
                }
            } => {},
            _ = snapshot.invalidated_at(cap_ms) => {},
            _ = async {
                match self.cap {
                    Some(cap) => tokio::time::sleep_until(cap.at.into()).await,
                    None => std::future::pending().await,
                }
            } => {},
        }
    }
}
