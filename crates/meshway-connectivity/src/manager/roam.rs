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
    /// Listeners bound to a specific address that is no longer assigned:
    /// closed, their bind retried ([`crate::manager`] listener retry).
    pub listeners_unbound: usize,
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
    ///   route's source toward its remote is retired and dialed again;
    /// - a listener bound to a specific address the host no longer has is
    ///   closed and its bind retried; every listener bind being retried runs
    ///   again now, its backoff starting over (the address may be back).
    pub async fn path_changed(&mut self) -> Result<Roamed, Error> {
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
        report.listeners_unbound = self.unbind_departed_listeners().await?;
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
                listeners_unbound = report.listeners_unbound,
                "Connectivity roam"
            );
        }
        Ok(report)
    }
}

/// The remotes this Node sends to now, in priority order: every selected
/// channel's remote, then the literal dial candidates (configured and from
/// facts) of the peers it may dial without a channel. The host's change
/// classification follows the route toward each ([`SocketFactory::observe_remotes`]).
fn remotes(shared: &Shared) -> Result<Vec<std::net::IpAddr>, Error> {
    let plan = shared.plan();
    let mut out = Vec::new();
    let connected: std::collections::BTreeSet<NodeId> = {
        let links = shared.links.lock().map_err(|_| Error::Worker)?;
        out.extend(links.values().map(|link| link.fact.remote.ip()));
        links.keys().copied().collect()
    };
    let now = chrono::Utc::now().timestamp_millis();
    for peer in plan.dial_peers() {
        if connected.contains(&peer) {
            continue;
        }
        let facts = shared.facts.get(peer);
        for (_, candidate) in plan.outgoing(peer)? {
            let network = if candidate.parameters.transport() == TransportProtocol::NativeUdp {
                meshway_model::state::SocketNetwork::Udp
            } else {
                meshway_model::state::SocketNetwork::Tcp
            };
            let learned = facts.iter().flat_map(|facts| {
                crate::plan::fact_endpoints(facts, &candidate.listener_key, network, now)
            });
            out.extend(
                candidate
                    .endpoints
                    .iter()
                    .cloned()
                    .chain(learned)
                    .filter_map(|endpoint| endpoint.host.parse::<std::net::IpAddr>().ok()),
            );
        }
    }
    Ok(out)
}

impl Manager {
    /// Close every listener whose specific bound address is no longer
    /// assigned and hand its key to the bind retry. A wildcard listener
    /// never moves; an address the host cannot answer for is kept.
    async fn unbind_departed_listeners(&mut self) -> Result<usize, Error> {
        let mut departed = Vec::new();
        for fact in &self.listeners {
            let ip = fact.local.ip();
            if !ip.is_unspecified() && self.factory.address_assigned(ip).await == Some(false) {
                departed.push((fact.key.clone(), fact.local));
            }
        }
        for (key, address) in &departed {
            let owners = self.retire_keys(&[resources::Key::Listener(key.clone())]);
            self.join_retired(owners).await?;
            tracing::warn!(key, %address, "Listener address left the host; closing it and retrying its bind");
            self.retry_listener(
                key.clone(),
                &Error::Bind {
                    key: key.clone(),
                    address: *address,
                    source: std::io::ErrorKind::AddrNotAvailable.into(),
                },
            )?;
        }
        if !departed.is_empty() {
            let _ = self.shared.control_events.try_send(Event::Changed);
        }
        Ok(departed.len())
    }

    /// Tell the host which remotes this Node sends to now.
    pub(super) fn observe_remotes(&self) {
        match remotes(&self.shared) {
            Ok(remotes) => self.factory.observe_remotes(remotes),
            Err(error) => tracing::debug!(%error, "Connectivity remotes not observed"),
        }
    }
}
