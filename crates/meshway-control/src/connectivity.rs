//! Validated, revocable UDP acquisition authority. Constructors stay with the
//! single Control owner; State observations are inputs, never grants.
use crate::{Control, Deadline, Error, ExecutionSnapshot, Started};
use meshway_configuration::execution::Carrier;
use meshway_model::{NodeId, RequestId, StateEpoch, state::SocketNetwork};
pub use meshway_protocol::connectivity::SocketReference;
use meshway_protocol::{StateQuery, StateView, connectivity::*};
use std::{net::SocketAddr, sync::Arc, time::Instant};
use tokio_util::sync::CancellationToken;

/// Issued authority, not a candidate cache. Every view of an immutable offer
/// shares its first deadline/cancellation, including a later query or cancel.
#[derive(Default)]
pub(super) struct Authority {
    issued: std::collections::BTreeMap<meshway_model::UdpTraversalId, ValidatedOffer>,
}

#[derive(Clone)]
pub struct DiscoveryContext {
    inner: Arc<DiscoveryAuthority>,
}
struct DiscoveryAuthority {
    snapshot: ExecutionSnapshot,
    epoch: StateEpoch,
    deadline: Deadline,
    servers: StunServers,
}

impl DiscoveryContext {
    pub fn snapshot(&self) -> &ExecutionSnapshot {
        &self.inner.snapshot
    }
    pub fn state_epoch(&self) -> StateEpoch {
        self.inner.epoch
    }
    pub fn servers(&self) -> &[StunServer] {
        self.inner.servers.as_slice()
    }
    pub fn expires_at_ms(&self) -> u64 {
        self.inner
            .deadline
            .expires_at_ms()
            .min(self.snapshot().expires_at_ms())
    }
    pub fn expires_at(&self) -> Instant {
        self.inner
            .deadline
            .expires_at()
            .min(self.snapshot().expires_at())
    }
    pub fn ensure_current(&self) -> Result<(), Error> {
        self.snapshot().ensure_current()?;
        if !self.inner.deadline.valid() {
            return Err(Error::Expired);
        }
        Ok(())
    }
    pub async fn invalidated(&self) {
        tokio::select! {
            _ = self.snapshot().invalidated_at(self.expires_at_ms()) => {},
            _ = tokio::time::sleep_until(self.expires_at().into()) => {},
        }
    }
}

#[derive(Clone)]
pub struct ValidatedOffer {
    inner: Arc<OfferAuthority>,
}
struct OfferAuthority {
    context: DiscoveryContext,
    offer: UdpTraversalOffer,
    deadline: Deadline,
    local_first: bool,
    cancelled: CancellationToken,
}

impl ValidatedOffer {
    pub fn offer(&self) -> &UdpTraversalOffer {
        &self.inner.offer
    }
    pub fn snapshot(&self) -> &ExecutionSnapshot {
        self.inner.context.snapshot()
    }
    pub fn local(&self) -> &UdpTraversalParticipant {
        if self.inner.local_first {
            &self.inner.offer.local
        } else {
            &self.inner.offer.peer
        }
    }
    pub fn peer(&self) -> &UdpTraversalParticipant {
        if self.inner.local_first {
            &self.inner.offer.peer
        } else {
            &self.inner.offer.local
        }
    }
    pub fn expires_at(&self) -> Instant {
        self.inner
            .deadline
            .expires_at()
            .min(self.inner.context.expires_at())
    }
    pub fn expires_at_ms(&self) -> u64 {
        self.inner
            .deadline
            .expires_at_ms()
            .min(self.inner.context.expires_at_ms())
    }
    pub fn ensure_current(&self) -> Result<(), Error> {
        self.inner.context.ensure_current()?;
        if self.inner.cancelled.is_cancelled() || !self.inner.deadline.valid() {
            return Err(Error::Expired);
        }
        meshway_policy::admission::admit_adjacency(
            self.snapshot().configuration()?,
            self.peer().candidate.socket.node_id,
            &self.inner.offer.carrier,
            self.snapshot().now_ms()?,
        )
        .map_err(|_| Error::Expired)?;
        Ok(())
    }
    pub fn cancel(&self) {
        self.inner.cancelled.cancel();
    }
    pub async fn invalidated(&self) {
        tokio::select! {
            _ = self.inner.cancelled.cancelled() => {},
            _ = self.snapshot().invalidated_at(self.expires_at_ms()) => {},
            _ = tokio::time::sleep_until(self.expires_at().into()) => {},
        }
    }
}

/// Active authority for bounded acquisition only. Clones share cancellation.
#[derive(Clone)]
pub struct AttemptScope(ValidatedOffer);
impl AttemptScope {
    pub fn validated_offer(&self) -> &ValidatedOffer {
        &self.0
    }
    pub fn ensure_current(&self) -> Result<(), Error> {
        self.0.ensure_current()
    }
    pub fn cancel(&self) {
        self.0.cancel();
    }
    pub async fn invalidated(&self) {
        self.0.invalidated().await;
    }
}

pub enum Outcome<T> {
    Current(T),
    Unavailable(UdpTraversalUnavailableReason),
}
pub struct Discovery {
    pub context: DiscoveryContext,
    pub attempts: Vec<AttemptView>,
}
pub enum AttemptView {
    Waiting(ValidatedOffer),
    Active(AttemptScope),
}

/// Numeric unicast recipient validation shared with the protected socket gate.
/// Private/site-local NAT addresses are valid; host-local control services are not.
pub fn udp_recipient(address: &str) -> Result<SocketAddr, Error> {
    let address: SocketAddr = address.parse().map_err(|_| Error::Contract)?;
    let invalid = address.port() == 0
        || address.ip().is_unspecified()
        || address.ip().is_multicast()
        || address.ip().is_loopback()
        || matches!(address, SocketAddr::V6(address) if address.scope_id() != 0 || address.flowinfo() != 0)
        || match address.ip() {
            std::net::IpAddr::V4(ip) => {
                ip.is_broadcast() || ip.is_link_local() || ip.octets()[0] == 0
            }
            std::net::IpAddr::V6(ip) => ip.is_unicast_link_local() || ip.to_ipv4_mapped().is_some(),
        };
    if invalid {
        Err(Error::Contract)
    } else {
        Ok(address)
    }
}

impl Control {
    fn connectivity_current(&self, snapshot: &ExecutionSnapshot) -> Result<(), Error> {
        self.token()?;
        snapshot.ensure_current()?;
        let current = self.execution_snapshot()?;
        if !snapshot.same_execution(&current) || snapshot.instance() != self.current_instance()? {
            return Err(Error::Expired);
        }
        Ok(())
    }

    pub async fn query_udp_traversal(
        &mut self,
        snapshot: &ExecutionSnapshot,
    ) -> Result<Outcome<Discovery>, Error> {
        self.connectivity_current(snapshot)?;
        let request = QueryUdpTraversalRequest {
            instance: snapshot.instance(),
            configuration: snapshot.reference(),
        };
        let started = Started::now(&self.clock)?;
        let response = self
            .client
            .query_udp_traversal(&request, self.token()?)
            .await;
        let response = self.registered(response)?;
        self.connectivity_current(snapshot)?;
        let QueryUdpTraversalResponse::Current {
            state_epoch,
            as_of_ms,
            expires_at_ms,
            stun_servers,
            attempts,
        } = response
        else {
            let QueryUdpTraversalResponse::Unavailable { reason } = response else {
                unreachable!()
            };
            for issued in self.connectivity.issued.values() {
                issued.cancel();
            }
            return Ok(Outcome::Unavailable(reason));
        };
        if expires_at_ms <= as_of_ms
            || stun_servers
                .as_slice()
                .iter()
                .enumerate()
                .any(|(index, server)| {
                    server.id.is_empty()
                        || server.id.len() > 128
                        || stun_servers.as_slice()[..index]
                            .iter()
                            .any(|old| old.id == server.id)
                })
        {
            self.fence();
            return Err(Error::Contract);
        }
        let context = DiscoveryContext {
            inner: Arc::new(DiscoveryAuthority {
                snapshot: snapshot.clone(),
                epoch: state_epoch,
                deadline: Deadline::from_response(started.clone(), expires_at_ms)?,
                servers: stun_servers,
            }),
        };
        let mut views = Vec::new();
        let visible: Vec<_> = attempts
            .as_slice()
            .iter()
            .map(|attempt| attempt.offer.attempt_id)
            .collect();
        for (id, issued) in &self.connectivity.issued {
            if !visible.contains(id) {
                issued.cancel();
            }
        }
        for attempt in attempts.into_vec() {
            // Query asOf belongs to this response. A prepare operation instead
            // uses its own immutable requested expiration, not older metadata.
            if attempt.offer.expires_at_ms > as_of_ms.saturating_add(MAX_UDP_TRAVERSAL_LIFETIME_MS)
            {
                self.fence();
                return Err(Error::Contract);
            }
            let offer =
                match self.validate_traversal_offer(&context, attempt.offer, started.clone()) {
                    Ok(offer) => offer,
                    Err(Error::Expired) => continue,
                    Err(error) => return Err(error),
                };
            views.push(match attempt.phase {
                UdpTraversalPhase::Waiting => AttemptView::Waiting(offer),
                UdpTraversalPhase::Active => AttemptView::Active(AttemptScope(offer)),
            });
        }
        Ok(Outcome::Current(Discovery {
            context,
            attempts: views,
        }))
    }

    fn validate_traversal_offer(
        &mut self,
        context: &DiscoveryContext,
        offer: UdpTraversalOffer,
        started: Started,
    ) -> Result<ValidatedOffer, Error> {
        self.connectivity_current(context.snapshot())?;
        context.ensure_current()?;
        let cfg = context.snapshot().configuration()?;
        let local_first = offer.local.candidate.socket.node_id == cfg.node_id;
        let (local, peer) = if local_first {
            (&offer.local, &offer.peer)
        } else {
            (&offer.peer, &offer.local)
        };
        let valid = offer.network_id == cfg.network_id
            && offer.state_epoch == context.state_epoch()
            && local.candidate.socket.node_id == cfg.node_id
            && local.candidate.socket.instance == context.snapshot().instance()
            && local.candidate.socket.configuration == context.snapshot().reference()
            && peer.candidate.socket.node_id != cfg.node_id
            && cfg.peers.contains_key(&peer.candidate.socket.node_id)
            && offer.carrier.protocol
                == meshway_configuration::transport::TransportProtocol::NativeUdp
            && [offer.carrier.dialer, offer.carrier.listener].contains(&cfg.node_id)
            && [offer.carrier.dialer, offer.carrier.listener]
                .contains(&peer.candidate.socket.node_id)
            && offer.compute_digest().map_err(|_| Error::Contract)? == offer.digest
            && udp_recipient(&local.address)?.is_ipv4() == udp_recipient(&peer.address)?.is_ipv4();
        if !valid {
            self.fence();
            return Err(Error::Contract);
        }
        let edge_deadline = meshway_policy::admission::admit_adjacency(
            cfg,
            peer.candidate.socket.node_id,
            &offer.carrier,
            context.snapshot().now_ms()?,
        )
        .map_err(|_| Error::Expired)?;
        if offer.expires_at_ms > edge_deadline
            || offer.expires_at_ms > context.snapshot().expires_at_ms()
        {
            self.fence();
            return Err(Error::Contract);
        }
        let now = self.clock.now()?;
        self.connectivity
            .issued
            .retain(|_, value| value.offer().expires_at_ms > now);
        if let Some(previous) = self.connectivity.issued.get(&offer.attempt_id) {
            if previous.offer() != &offer {
                self.fence();
                return Err(Error::Contract);
            }
            previous.ensure_current()?;
            return Ok(previous.clone());
        }
        if self.connectivity.issued.len() >= MAX_UDP_TRAVERSAL_ATTEMPTS {
            return Err(Error::Pending);
        }
        let value = ValidatedOffer {
            inner: Arc::new(OfferAuthority {
                context: context.clone(),
                deadline: Deadline::from_response(started, offer.expires_at_ms)?,
                offer,
                local_first,
                cancelled: CancellationToken::new(),
            }),
        };
        value.ensure_current()?;
        self.connectivity
            .issued
            .insert(value.offer().attempt_id, value.clone());
        Ok(value)
    }

    pub async fn query_connectivity_peers(
        &mut self,
        context: &DiscoveryContext,
        peers: &[NodeId],
    ) -> Result<StateView, Error> {
        self.connectivity_current(context.snapshot())?;
        context.ensure_current()?;
        let cfg = context.snapshot().configuration()?;
        if peers.is_empty()
            || peers.len() > 8
            || peers
                .iter()
                .enumerate()
                .any(|(i, peer)| !cfg.peers.contains_key(peer) || peers[..i].contains(peer))
        {
            return Err(Error::Contract);
        }
        let response = self
            .client
            .query_peer_state(
                &StateQuery {
                    nodes: Some(peers.to_vec()),
                },
                self.token()?,
            )
            .await;
        let view = self.registered(response)?;
        self.connectivity_current(context.snapshot())?;
        context.ensure_current()?;
        if view.network_id != context.snapshot().configuration()?.network_id
            || view.nodes.keys().any(|node| !peers.contains(node))
        {
            self.fence();
            return Err(Error::Contract);
        }
        // The Server owns freshness/disclosure. Validate every disclosed binding
        // before exposing it; these facts still cannot authorize a UDP send.
        for (node, view) in &view.nodes {
            if let Some(meshway_protocol::state::VisibleReport::Peer { report }) = &view.report {
                for endpoint in &report.endpoints {
                    if endpoint.network != SocketNetwork::Udp {
                        continue;
                    }
                    let reference = &endpoint.candidate_reference.socket;
                    if reference.node_id != *node
                        || Some(reference.instance.instance_id) != view.instance_id
                        || reference.socket_id != endpoint.socket_id
                        || reference.socket_generation != endpoint.generation
                        || reference.resource != endpoint.resource
                        || report.configuration.applied.as_ref() != Some(&reference.configuration)
                    {
                        self.fence();
                        return Err(Error::Contract);
                    }
                }
            }
        }
        Ok(view)
    }

    pub async fn prepare_udp_traversal(
        &mut self,
        context: &DiscoveryContext,
        request: &PrepareUdpTraversalRequest,
    ) -> Result<Outcome<ValidatedOffer>, Error> {
        self.connectivity_current(context.snapshot())?;
        context.ensure_current()?;
        if request.instance != context.snapshot().instance()
            || request.configuration != context.snapshot().reference()
            || request.state_epoch != context.state_epoch()
            || request.local_candidate.socket.instance != request.instance
            || request.local_candidate.socket.configuration != request.configuration
            || request.local_candidate.socket.node_id != context.snapshot().configuration()?.node_id
            || request.peer_candidate.socket.node_id != request.peer_id
            || request.expires_at_ms > context.expires_at_ms()
            || request.expires_at_ms
                > self
                    .clock
                    .now()?
                    .saturating_add(MAX_UDP_TRAVERSAL_LIFETIME_MS)
        {
            return Err(Error::Contract);
        }
        meshway_policy::admission::admit_adjacency(
            context.snapshot().configuration()?,
            request.peer_id,
            &request.carrier,
            context.snapshot().now_ms()?,
        )
        .map_err(|_| Error::Expired)?;
        let started = Started::now(&self.clock)?;
        let response = self
            .client
            .prepare_udp_traversal(request, self.token()?)
            .await;
        let response = self.registered(response)?;
        self.connectivity_current(context.snapshot())?;
        match response {
            PrepareUdpTraversalResponse::Unavailable { reason } => Ok(Outcome::Unavailable(reason)),
            PrepareUdpTraversalResponse::Offer { offer } => {
                if offer.request_id != request.request_id
                    || offer.carrier != request.carrier
                    || offer.local.candidate != request.local_candidate
                    || offer.peer.candidate != request.peer_candidate
                    || offer.expires_at_ms > request.expires_at_ms
                {
                    self.fence();
                    return Err(Error::Contract);
                }
                Ok(Outcome::Current(
                    self.validate_traversal_offer(context, offer, started)?,
                ))
            }
        }
    }

    pub async fn accept_udp_traversal(
        &mut self,
        offered: &ValidatedOffer,
    ) -> Result<Outcome<AttemptView>, Error> {
        self.connectivity_current(offered.snapshot())?;
        offered.ensure_current()?;
        let path = meshway_control_client::AcceptUdpTraversalPath {
            attempt_id: offered.offer().attempt_id,
        };
        let response = self
            .client
            .accept_udp_traversal(&path, &attempt_request(offered), self.token()?)
            .await;
        let response = self.registered(response)?;
        self.connectivity_current(offered.snapshot())?;
        offered.ensure_current()?;
        let (offer, active) = match response {
            AcceptUdpTraversalResponse::Unavailable { reason } => {
                offered.cancel();
                return Ok(Outcome::Unavailable(reason));
            }
            AcceptUdpTraversalResponse::Waiting { offer } => (offer, false),
            AcceptUdpTraversalResponse::Active { offer } => (offer, true),
        };
        if &offer != offered.offer() {
            self.fence();
            return Err(Error::Contract);
        }
        Ok(Outcome::Current(if active {
            AttemptView::Active(AttemptScope(offered.clone()))
        } else {
            AttemptView::Waiting(offered.clone())
        }))
    }

    pub async fn cancel_udp_traversal(
        &mut self,
        offered: &ValidatedOffer,
    ) -> Result<CancelUdpTraversalResponse, Error> {
        offered.cancel();
        if let Some(issued) = self.connectivity.issued.get(&offered.offer().attempt_id) {
            issued.cancel();
        }
        self.connectivity_current(offered.snapshot())?;
        let path = meshway_control_client::CancelUdpTraversalPath {
            attempt_id: offered.offer().attempt_id,
        };
        let response = self
            .client
            .cancel_udp_traversal(&path, &attempt_request(offered), self.token()?)
            .await;
        self.registered(response)
    }
}

fn attempt_request(value: &ValidatedOffer) -> UdpTraversalAttemptRequest {
    UdpTraversalAttemptRequest {
        instance: value.snapshot().instance(),
        configuration: value.snapshot().reference(),
        state_epoch: value.offer().state_epoch,
        version: value.offer().version,
        digest: value.offer().digest,
    }
}

/// Construct immutable retry input once. It contains no authority by itself.
pub fn prepare_request(
    context: &DiscoveryContext,
    carrier: Carrier,
    local: CandidateReference,
    peer: CandidateReference,
) -> Result<PrepareUdpTraversalRequest, Error> {
    context.ensure_current()?;
    Ok(PrepareUdpTraversalRequest {
        instance: context.snapshot().instance(),
        configuration: context.snapshot().reference(),
        state_epoch: context.state_epoch(),
        request_id: RequestId::new(crate::random_id()?).map_err(|_| Error::Cryptography)?,
        expires_at_ms: context.expires_at_ms().min(
            context
                .snapshot()
                .now_ms()?
                .saturating_add(MAX_UDP_TRAVERSAL_LIFETIME_MS),
        ),
        peer_id: peer.socket.node_id,
        carrier,
        local_candidate: local,
        peer_candidate: peer,
    })
}

impl std::fmt::Debug for AttemptScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AttemptScope")
            .field("id", &self.0.offer().attempt_id)
            .finish_non_exhaustive()
    }
}
impl std::fmt::Debug for DiscoveryContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DiscoveryContext").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::udp_recipient;

    #[test]
    fn acquisition_recipients_reject_local_services_and_scoped_aliases() {
        for address in [
            "localhost:53",
            "127.0.0.1:53",
            "[::1]:53",
            "0.0.0.0:1",
            "[::]:1",
            "224.0.0.1:1",
            "255.255.255.255:1",
            "169.254.1.1:1",
            "[fe80::1]:1",
            "[2001:db8::1%2]:1",
            "[::ffff:192.0.2.1]:1",
            "192.0.2.1:0",
        ] {
            assert!(udp_recipient(address).is_err(), "{address}");
        }
        for address in [
            "192.168.1.2:4567",
            "10.0.0.1:4567",
            "198.51.100.1:4567",
            "[2001:db8::1]:4567",
        ] {
            assert!(udp_recipient(address).is_ok(), "{address}");
        }
    }
}
