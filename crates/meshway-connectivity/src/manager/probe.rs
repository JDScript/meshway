//! Local network diagnostics over the existing observations: a request makes
//! every traversal socket run its ordinary same-socket STUN round now, then
//! the caller reads what those rounds observed. No other socket or message
//! is created, and nothing here changes State, candidates or authority.
use super::{Manager, Shared};
use crate::Error;
use meshway_model::TransportProtocol;
use std::{collections::BTreeMap, net::SocketAddr, sync::Arc};
use tokio::time::{Duration, Instant};

/// The latest validated STUN answer of one socket from one observer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StunSample {
    /// Mapped address the observer saw.
    pub mapped: SocketAddr,
    /// Round trip of a transaction answered after a single send; `None`
    /// after a retransmission, whose round trip is ambiguous.
    pub rtt: Option<Duration>,
    pub at: Instant,
}

/// Latest answers of every socket by observer address.
pub(super) type StunSamples = BTreeMap<[u8; 32], BTreeMap<SocketAddr, StunSample>>;

/// One traversal socket and its observations.
#[derive(Clone, Debug)]
pub struct SocketObservation {
    pub socket: super::SocketFact,
    /// By observer address.
    pub samples: BTreeMap<SocketAddr, StunSample>,
}

#[derive(Clone, Debug)]
pub struct StunReport {
    /// When the request was made; samples at or after it are this round's.
    pub requested_at: Instant,
    pub sockets: Vec<SocketObservation>,
    /// Authenticated Native UDP channels selected now.
    pub udp_links: usize,
}

/// A pending local observation request of one Manager.
pub struct StunProbe {
    shared: Arc<Shared>,
    requested_at: Instant,
}

impl Manager {
    /// Ask every traversal socket to observe its mapping now, within the
    /// Manager's existing query budget and STUN metadata.
    pub fn stun_probe(&self) -> Result<StunProbe, Error> {
        self.check_view()?;
        let requested_at = Instant::now();
        *self
            .shared
            .observation_request
            .lock()
            .map_err(|_| Error::Worker)? = Some(requested_at);
        Ok(StunProbe {
            shared: self.shared.clone(),
            requested_at,
        })
    }
}

impl StunProbe {
    /// Wait until every traversal socket answered after the request, the
    /// Manager stopped, or `timeout` passed; then report what is known.
    pub async fn wait(self, timeout: Duration) -> StunReport {
        let deadline = self.requested_at + timeout;
        loop {
            let report = self.report();
            let answered = !report.sockets.is_empty()
                && report.sockets.iter().all(|socket| {
                    socket
                        .samples
                        .values()
                        .any(|sample| sample.at >= self.requested_at)
                });
            if answered || self.shared.stop.is_cancelled() || Instant::now() >= deadline {
                return report;
            }
            tokio::select! {
                _ = self.shared.stop.cancelled() => {},
                _ = tokio::time::sleep(Duration::from_millis(100).min(deadline - Instant::now())) => {},
            }
        }
    }

    fn report(&self) -> StunReport {
        let endpoints: Vec<[u8; 32]> = self
            .shared
            .traversal_endpoints
            .lock()
            .map(|endpoints| endpoints.keys().copied().collect())
            .unwrap_or_default();
        let sockets = self
            .shared
            .sockets
            .lock()
            .map(|s| s.clone())
            .unwrap_or_default();
        let samples = self
            .shared
            .stun_samples
            .lock()
            .map(|s| s.clone())
            .unwrap_or_default();
        let udp_links = self
            .shared
            .links
            .lock()
            .map(|links| {
                links
                    .values()
                    .filter(|link| {
                        link.fact.carrier.protocol == TransportProtocol::NativeUdp
                            && link.scope.check().is_ok()
                    })
                    .count()
            })
            .unwrap_or(0);
        StunReport {
            requested_at: self.requested_at,
            sockets: endpoints
                .iter()
                .filter_map(|id| {
                    Some(SocketObservation {
                        socket: sockets.get(id)?.clone(),
                        samples: samples.get(id).cloned().unwrap_or_default(),
                    })
                })
                .collect(),
            udp_links,
        }
    }
}
