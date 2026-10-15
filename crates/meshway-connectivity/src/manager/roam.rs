//! The Manager's reaction to an underlay path change (roaming). The
//! Underlay classifies the change; this owner only acts on its own sockets
//! and channels. Identity, content and leases are untouched.
use super::*;
use meshway_model::state::EndpointSource;

/// What one path change did, for the host's diagnostics.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Roamed {
    /// Native UDP channels asked to prove themselves at the roaming pace.
    pub datagram_channels: usize,
    /// Stream channels retired because their local source left the route.
    pub streams_retired: usize,
    /// STUN observations made on the previous path, withdrawn.
    pub observations_withdrawn: usize,
}

impl Manager {
    /// The Underlay classified a path change: this Node's sockets may now
    /// reach peers from another source.
    ///
    /// - STUN observations of the old path are withdrawn at once (the next
    ///   State report no longer carries them) and every traversal socket
    ///   observes again now;
    /// - every datagram owner sends each channel's health Ping at once and
    ///   then at the roaming pace, so a peer sees the new source within a
    ///   round trip and challenges it;
    /// - candidate pacing of every peer starts over (`own_network`);
    /// - a stream channel whose local address is no longer the marked
    ///   route's source toward its remote is retired and dialed again.
    pub async fn path_changed(&self) -> Result<Roamed, Error> {
        self.check_view()?;
        let mut report = Roamed::default();
        {
            let mut sockets = self.shared.sockets.lock().map_err(|_| Error::Worker)?;
            for socket in sockets.values_mut() {
                let before = socket.endpoints.len();
                socket
                    .endpoints
                    .retain(|endpoint| !matches!(endpoint.origin, EndpointSource::Stun { .. }));
                report.observations_withdrawn += before - socket.endpoints.len();
            }
        }
        self.shared
            .stun_confirmed
            .lock()
            .map_err(|_| Error::Worker)?
            .clear();
        *self
            .shared
            .roam_observation
            .lock()
            .map_err(|_| Error::Worker)? = Some(tokio::time::Instant::now());
        let links: Vec<_> = self
            .shared
            .links
            .lock()
            .map_err(|_| Error::Worker)?
            .values()
            .filter(|link| link.scope.check().is_ok())
            .map(|link| link.fact.clone())
            .collect();
        self.shared
            .roams
            .send_modify(|epoch| *epoch = epoch.wrapping_add(1));
        for fact in links {
            if fact.carrier.protocol == TransportProtocol::NativeUdp {
                report.datagram_channels += 1;
                continue;
            }
            let source = self.factory.route_source(fact.remote).await;
            if source != Some(fact.local.ip()) {
                retire_for(&self.shared, &fact, "route_changed")?;
                report.streams_retired += 1;
            }
        }
        if report.observations_withdrawn != 0 {
            let _ = self.shared.control_events.try_send(Event::Changed);
        }
        if diagnostic_event() {
            tracing::debug!(
                operation = "roam",
                cause = "path_changed",
                datagram_channels = report.datagram_channels,
                streams_retired = report.streams_retired,
                observations_withdrawn = report.observations_withdrawn,
                "Connectivity roam"
            );
        }
        Ok(report)
    }
}
