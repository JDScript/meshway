//! The host executes discovery with its unique `Control::query_dns` driver.
//! Neither this owner nor a discovery result can authorize resolver traffic.
use crate::{
    ConfigurationError, ExitSelection, LocalResolverRoute, PublishedDns, Query, ResponseTransport,
    Selection, SelectionUnavailable, TrustedUpstreamResponse, WireError, encode_response,
};
use bytes::Bytes;
use hickory_proto::{
    op::{Message, ResponseCode},
    rr::{RData, Record, RecordType, rdata},
};
use meshway_configuration::{
    NodeAddresses, execution::NodeConfiguration, node_dns_name, normalize_dns_name,
};
use meshway_control::ExecutionSnapshot;
use meshway_model::{NetworkId, NodeId, RequestId, Revision};
use meshway_protocol::NodeDnsQueryResponse;
use std::{
    collections::BTreeMap,
    fmt,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

/// Explicit resource bounds. Discovery has no cache: its contract has no TTL.
/// The timeout bounds work only; it never extends the control snapshot lifetime.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_pending: usize,
    pub query_timeout: Duration,
}

#[derive(Debug, thiserror::Error)]
pub enum OwnerError {
    #[error("DNS limits require nonzero capacity and a representable nonzero timeout")]
    Limits,
    #[error("DNS execution snapshot is not current")]
    Inactive,
    #[error("DNS refresh requires the same execution epoch, instance and exact target")]
    DifferentExecution,
    #[error(transparent)]
    Configuration(#[from] ConfigurationError),
    #[error("DNS output is cancelled or expired")]
    Cancelled,
    #[error(transparent)]
    Wire(#[from] WireError),
}

/// Local failures do not imply a DNS negative answer or reveal Access policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unavailable {
    Inactive,
    Capacity,
    InvalidDiscoveryName,
    UnsupportedRecordType,
    MissingResolverPlan,
    MissingExitPlan,
    InvalidTicket,
    Expired,
    BindingMismatch,
    /// The response cannot be bound to these execution settings. Reload before
    /// retrying; a newer Server revision is not necessarily an invalid response.
    ReloadRequired,
    /// Unknown and forbidden are deliberately indistinguishable. Valid discovery
    /// maps this to REFUSED, without SOA, AA or a negative cache entry.
    NotVisible,
}

/// Opaque owner-generation and operation identity, unrelated to DNS request IDs
/// and HTTP request IDs. Clones permit cancellation, not repeated completion.
#[derive(Clone)]
pub struct Ticket {
    generation: Arc<()>,
    sequence: u64,
    cancelled: CancellationToken,
    deadline: Instant,
}

impl Ticket {
    /// Cancel work and any unsent answer, even if a dispatcher future is dropped.
    /// The owner reclaims the bounded pending slot on its next `reap`/operation.
    pub fn cancel(&self) {
        self.cancelled.cancel();
    }
    /// Query work deadline, already capped by the original snapshot expiry.
    /// This is not execution authority and cannot extend that snapshot.
    pub fn expires_at(&self) -> Instant {
        self.deadline
    }

    /// Select this and `Owner::invalidated()` around host-driven discovery I/O.
    /// No task is spawned; explicit cancel/replacement wakes this waiter.
    pub async fn cancelled(&self) {
        tokio::select! { biased;
            _ = self.cancelled.cancelled() => {},
            _ = tokio::time::sleep_until(self.deadline.into()) => {},
        }
    }
}

impl fmt::Debug for Ticket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Ticket").finish_non_exhaustive()
    }
}

pub struct DiscoveryRequest {
    pub ticket: Ticket,
    /// Canonical exact name for `Control::query_dns`; never an inverse guess.
    pub name: String,
}

/// Work for one exact configured resolver. The route is created only from the
/// current published local permit; the host's mapping only chooses a demand ID.
#[derive(Debug)]
pub struct UpstreamWork {
    pub(crate) ticket: Ticket,
    pub(crate) route: LocalResolverRoute,
    pub(crate) query: Query,
}

impl UpstreamWork {
    pub fn ticket(&self) -> &Ticket {
        &self.ticket
    }
    pub fn route(&self) -> &LocalResolverRoute {
        &self.route
    }
    pub fn query(&self) -> &Query {
        &self.query
    }
}

impl fmt::Debug for DiscoveryRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DiscoveryRequest").finish_non_exhaustive()
    }
}

#[derive(Debug)]
pub enum Begin {
    Answer(Answer),
    DiscoveryRequest(DiscoveryRequest),
    Upstream(Box<UpstreamWork>),
    /// Let the host preserve ordinary system resolution, without a socket grant.
    Unmanaged,
    Unavailable(Unavailable),
}

/// An answer remains bound to the original control snapshot, owner generation,
/// query deadline and cancellation. Check immediately before sending, and select
/// `invalidated()` around blocked output. Encoding does not authorize later I/O.
pub struct Answer {
    snapshot: Box<ExecutionSnapshot>,
    ready: Box<ReadyAnswer>,
    scope_fence: Option<crate::dispatcher::ScopeFence>,
}

impl fmt::Debug for Answer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Answer").finish_non_exhaustive()
    }
}

impl Answer {
    pub(crate) fn answers(&self, query: &Query) -> bool {
        self.ready.query == *query
    }

    pub(crate) fn with_scope_fence(mut self, fence: crate::dispatcher::ScopeFence) -> Self {
        self.scope_fence = Some(fence);
        self
    }

    pub fn ensure_current(&self) -> Result<(), OwnerError> {
        self.snapshot
            .ensure_current()
            .map_err(|_| OwnerError::Inactive)?;
        if self.ready.cancelled.is_cancelled() || Instant::now() >= self.ready.deadline {
            return Err(OwnerError::Cancelled);
        }
        if self
            .scope_fence
            .as_ref()
            .is_some_and(|fence| !fence.current())
        {
            return Err(OwnerError::Cancelled);
        }
        if let Some(route) = &self.ready.route {
            route.ensure_current().map_err(|_| OwnerError::Cancelled)?;
        }
        Ok(())
    }

    pub fn encode(
        &self,
        transport: ResponseTransport,
        udp_payload_limit: u16,
    ) -> Result<Bytes, OwnerError> {
        self.ensure_current()?;
        let bytes = encode_response(
            &self.ready.query,
            &self.ready.message,
            transport,
            udp_payload_limit,
        )?;
        self.ensure_current()?;
        Ok(bytes)
    }

    pub async fn invalidated(&self) {
        if self.ensure_current().is_err() {
            return;
        }
        tokio::select! { biased;
            _ = self.snapshot.invalidated() => {},
            _ = self.ready.cancelled.cancelled() => {},
            _ = tokio::time::sleep_until(self.ready.deadline.into()) => {},
            _ = async {
                match &self.ready.route {
                    Some(route) => route.invalidated().await,
                    None => std::future::pending().await,
                }
            } => {},
            _ = async {
                match &self.scope_fence {
                    Some(fence) => fence.invalidated().await,
                    None => std::future::pending().await,
                }
            } => {},
        }
    }
}

/// One bounded query owner per installed snapshot. No token, HTTP client, ACL,
/// lease renewal, background task, upstream socket or domain grant lives here.
/// Published and discovered records use TTL 0: no separate record TTL authority
/// is currently supplied. Pending queries are the only retained query data.
pub struct Owner {
    snapshot: ExecutionSnapshot,
    state: State,
}

impl fmt::Debug for Owner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Owner")
            .field("pending", &self.state.pending.len())
            .finish_non_exhaustive()
    }
}

impl Owner {
    pub fn new(snapshot: ExecutionSnapshot, limits: Limits) -> Result<Self, OwnerError> {
        if limits.max_pending == 0
            || limits.query_timeout.is_zero()
            || Instant::now().checked_add(limits.query_timeout).is_none()
        {
            return Err(OwnerError::Limits);
        }
        let configuration = snapshot.configuration().map_err(|_| OwnerError::Inactive)?;
        let state = State::new(configuration, limits)?;
        snapshot
            .ensure_current()
            .map_err(|_| OwnerError::Inactive)?;
        Ok(Self { snapshot, state })
    }

    /// Replacing even the same snapshot invalidates all previous tickets and
    /// unsent answers. Failed construction leaves the existing owner intact.
    pub fn replace(&mut self, snapshot: ExecutionSnapshot) -> Result<(), OwnerError> {
        *self = Self::new(snapshot, self.state.limits)?;
        Ok(())
    }

    /// Explicit same-execution renewal for future queries. Both snapshots must
    /// still be current; a stopped owner cannot be refreshed. Unlike `replace`,
    /// this requires the same control epoch, instance, network, node and target.
    ///
    /// Conservatively cancels every pending ticket and unsent answer, including
    /// published answers, before installing the new guard. No old operation can
    /// complete under renewed deadlines or bypass its original wall-clock cap.
    /// Failed validation leaves the owner unchanged.
    pub fn refresh(&mut self, snapshot: ExecutionSnapshot) -> Result<(), OwnerError> {
        snapshot
            .ensure_current()
            .map_err(|_| OwnerError::Inactive)?;
        self.snapshot
            .ensure_current()
            .map_err(|_| OwnerError::Inactive)?;
        if self.state.cancelled.is_cancelled() {
            return Err(OwnerError::Inactive);
        }
        if !self.snapshot.same_execution(&snapshot) {
            return Err(OwnerError::DifferentExecution);
        }
        self.state.refresh_generation();
        self.snapshot = snapshot;
        Ok(())
    }

    pub fn begin(&mut self, query: Query, exit: ExitSelection) -> Begin {
        if self.snapshot.ensure_current().is_err() {
            self.state.stop();
            return Begin::Unavailable(Unavailable::Inactive);
        }
        let outcome = self
            .state
            .begin(query, exit, Instant::now(), self.snapshot.expires_at());
        self.finish(outcome)
    }

    /// Authoritative live entry. Profile strings and caller booleans never
    /// establish no-exit operation. Unsupported exit plans still allow MagicDNS.
    pub fn begin_published(
        &mut self,
        query: Query,
        resolver_demands: &BTreeMap<String, RequestId>,
    ) -> Begin {
        self.begin_published_until(query, resolver_demands, self.snapshot.expires_at())
    }

    /// Additionally cap every ticket, route and answer by the original client's
    /// request deadline. A host may shorten, but never extend, owner authority.
    pub fn begin_published_until(
        &mut self,
        query: Query,
        resolver_demands: &BTreeMap<String, RequestId>,
        request_deadline: Instant,
    ) -> Begin {
        use meshway_configuration::execution::DnsEgress;
        self.reap();
        if Instant::now() >= request_deadline {
            return Begin::Unavailable(Unavailable::Expired);
        }
        let configuration = match self.snapshot.configuration() {
            Ok(configuration) => configuration,
            Err(_) => return Begin::Unavailable(Unavailable::Inactive),
        };
        let no_exit = matches!(configuration.dns.egress, DnsEgress::NoExit);
        let exit = if no_exit {
            ExitSelection::NoExit
        } else {
            ExitSelection::Exit
        };
        let selected = self.state.published.select(&query, exit);
        let Selection::Resolvers(candidates) = selected else {
            let mut begin = self.begin(query, exit);
            match &mut begin {
                Begin::Answer(answer) => {
                    answer.ready.deadline = answer.ready.deadline.min(request_deadline)
                }
                Begin::DiscoveryRequest(request) => {
                    request.ticket.deadline = request.ticket.deadline.min(request_deadline);
                    if let Some(pending) = self.state.pending.get_mut(&request.ticket.sequence) {
                        pending.deadline = pending.deadline.min(request_deadline);
                    }
                }
                _ => {}
            }
            return begin;
        };
        if !no_exit {
            return Begin::Unavailable(Unavailable::MissingExitPlan);
        }
        let now = Instant::now();
        let Some(deadline) = now
            .checked_add(self.state.limits.query_timeout)
            .map(|d| d.min(self.snapshot.expires_at()).min(request_deadline))
        else {
            return Begin::Unavailable(Unavailable::Expired);
        };
        // Preserve configured order. A missing/denied first resolver never
        // silently widens selection to another endpoint.
        let Some(candidate) = candidates.first() else {
            return Begin::Unavailable(Unavailable::MissingResolverPlan);
        };
        let Some(id) = resolver_demands.get(candidate.key) else {
            return Begin::Unavailable(Unavailable::MissingResolverPlan);
        };
        let mut route =
            match LocalResolverRoute::new(self.snapshot.clone(), candidate.key, *id, deadline) {
                Ok(route) => route,
                Err(crate::RouteError::Expired) => return Begin::Unavailable(Unavailable::Expired),
                Err(crate::RouteError::Binding) => {
                    return Begin::Unavailable(Unavailable::MissingResolverPlan);
                }
            };
        let ticket = match self.state.insert_pending(
            query.clone(),
            String::new(),
            PendingKind::Upstream,
            deadline,
        ) {
            Ok(ticket) => ticket,
            Err(reason) => return Begin::Unavailable(reason),
        };
        route.bind_cancellation(ticket.cancelled.clone());
        Begin::Upstream(Box::new(UpstreamWork {
            ticket,
            route,
            query,
        }))
    }

    /// A response cannot be transplanted across owner generations or renewed
    /// grants. The original route deadline also bounds completed answer I/O.
    pub fn complete_upstream(
        &mut self,
        work: &UpstreamWork,
        response: TrustedUpstreamResponse,
    ) -> Begin {
        if self.snapshot.ensure_current().is_err() || work.route.ensure_current().is_err() {
            self.cancel(&work.ticket);
            return Begin::Unavailable(Unavailable::Expired);
        }
        let pending = match self.state.take_pending(&work.ticket, Instant::now()) {
            Ok(pending) => pending,
            Err(reason) => return Begin::Unavailable(reason),
        };
        if pending.kind != PendingKind::Upstream
            || response.query != pending.query
            || response.message.metadata.id != pending.query.id()
            || response.message.queries.as_slice() != std::slice::from_ref(pending.query.question())
        {
            pending.cancelled.cancel();
            return Begin::Unavailable(Unavailable::BindingMismatch);
        }
        let ready = Box::new(ReadyAnswer {
            query: pending.query,
            message: response.message,
            cancelled: pending.cancelled,
            deadline: pending.deadline,
            route: Some(work.route.clone()),
        });
        self.finish(Outcome::Answer(ready))
    }

    /// Pass only the typed result returned by the existing `Control::query_dns`
    /// operation for this request. That generated-client path owns authentication
    /// and final Server checks. This method also verifies execution binding and
    /// the local monotonic query deadline. Server `asOf` is informational: it
    /// supplies neither lifetime authority nor a client-clock synchronization
    /// guarantee. Its date-time representation is checked by the typed decoder.
    /// A completion consumes its ticket even if the response fails validation.
    pub fn complete_discovery(&mut self, ticket: &Ticket, response: NodeDnsQueryResponse) -> Begin {
        if self.snapshot.ensure_current().is_err() {
            self.state.stop();
            return Begin::Unavailable(Unavailable::Inactive);
        }
        let outcome = self.state.complete(ticket, response, Instant::now());
        self.finish(outcome)
    }

    /// Invalidate pending work and any completed but unsent answer for this
    /// ticket. Returns whether a pending slot was removed.
    pub fn cancel(&mut self, ticket: &Ticket) -> bool {
        self.state.cancel(ticket)
    }

    /// Stop is terminal for this generation. A new snapshot can be installed
    /// explicitly with `replace`; old tickets and answers remain invalid.
    pub fn stop(&mut self) {
        self.state.stop();
    }

    /// Reclaim expired pending slots without creating a task or timer owner.
    pub fn reap(&mut self) {
        if self.snapshot.ensure_current().is_err() {
            self.state.stop();
        }
        self.state.reap(Instant::now());
    }

    pub fn pending_len(&self) -> usize {
        self.state.pending.len()
    }

    pub async fn invalidated(&self) {
        tokio::select! { biased;
            _ = self.snapshot.invalidated() => {},
            _ = self.state.cancelled.cancelled() => {},
        }
    }

    fn finish(&mut self, outcome: Outcome) -> Begin {
        if self.snapshot.ensure_current().is_err() {
            self.state.stop();
            return Begin::Unavailable(Unavailable::Inactive);
        }
        match outcome {
            Outcome::Answer(ready) => Begin::Answer(Answer {
                snapshot: Box::new(self.snapshot.clone()),
                ready,
                scope_fence: None,
            }),
            Outcome::Discovery(request) => Begin::DiscoveryRequest(request),
            Outcome::Unmanaged => Begin::Unmanaged,
            Outcome::Unavailable(reason) => Begin::Unavailable(reason),
        }
    }
}

struct Binding {
    network: NetworkId,
    source: NodeId,
    revision: Revision,
    self_name: String,
    self_addresses: NodeAddresses,
    peers: BTreeMap<NodeId, NodeAddresses>,
}

struct State {
    binding: Binding,
    published: PublishedDns,
    limits: Limits,
    generation: Arc<()>,
    cancelled: CancellationToken,
    sequence: u64,
    pending: BTreeMap<u64, Pending>,
}

struct Pending {
    query: Query,
    name: String,
    deadline: Instant,
    cancelled: CancellationToken,
    kind: PendingKind,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PendingKind {
    Discovery,
    Upstream,
}

struct ReadyAnswer {
    query: Query,
    message: Message,
    cancelled: CancellationToken,
    deadline: Instant,
    route: Option<LocalResolverRoute>,
}

enum Outcome {
    Answer(Box<ReadyAnswer>),
    Discovery(DiscoveryRequest),
    Unmanaged,
    Unavailable(Unavailable),
}

impl State {
    fn insert_pending(
        &mut self,
        query: Query,
        name: String,
        kind: PendingKind,
        deadline: Instant,
    ) -> Result<Ticket, Unavailable> {
        if self.cancelled.is_cancelled() {
            return Err(Unavailable::Inactive);
        }
        if Instant::now() >= deadline {
            return Err(Unavailable::Expired);
        }
        if self.pending.len() >= self.limits.max_pending {
            return Err(Unavailable::Capacity);
        }
        let Some(sequence) = self.sequence.checked_add(1) else {
            self.stop();
            return Err(Unavailable::Inactive);
        };
        self.sequence = sequence;
        let ticket = Ticket {
            generation: self.generation.clone(),
            sequence,
            cancelled: self.cancelled.child_token(),
            deadline,
        };
        self.pending.insert(
            sequence,
            Pending {
                query,
                name,
                deadline,
                kind,
                cancelled: ticket.cancelled.clone(),
            },
        );
        Ok(ticket)
    }

    fn take_pending(&mut self, ticket: &Ticket, now: Instant) -> Result<Pending, Unavailable> {
        if self.cancelled.is_cancelled() {
            return Err(Unavailable::Inactive);
        }
        if !Arc::ptr_eq(&self.generation, &ticket.generation) || ticket.cancelled.is_cancelled() {
            return Err(Unavailable::InvalidTicket);
        }
        let pending = self
            .pending
            .remove(&ticket.sequence)
            .ok_or(Unavailable::InvalidTicket)?;
        if now >= pending.deadline {
            pending.cancelled.cancel();
            return Err(Unavailable::Expired);
        }
        Ok(pending)
    }
    fn new(configuration: &NodeConfiguration, limits: Limits) -> Result<Self, ConfigurationError> {
        Ok(Self {
            binding: Binding {
                network: configuration.network_id,
                source: configuration.node_id,
                revision: configuration.revision,
                self_name: node_dns_name(
                    &configuration.hostname,
                    &configuration.dns.settings.domain,
                )
                .map_err(|_| ConfigurationError::Name)?,
                self_addresses: configuration.addresses.clone(),
                peers: configuration
                    .peers
                    .iter()
                    .map(|(id, peer)| (*id, peer.addresses.clone()))
                    .collect(),
            },
            published: PublishedDns::from_execution(&configuration.dns)?,
            limits,
            generation: Arc::new(()),
            cancelled: CancellationToken::new(),
            sequence: 0,
            pending: BTreeMap::new(),
        })
    }

    fn begin(
        &mut self,
        query: Query,
        exit: ExitSelection,
        now: Instant,
        expires_at: Instant,
    ) -> Outcome {
        self.reap(now);
        if self.cancelled.is_cancelled() {
            return Outcome::Unavailable(Unavailable::Inactive);
        }
        if now >= expires_at {
            return Outcome::Unavailable(Unavailable::Expired);
        }
        let Some(deadline) = now
            .checked_add(self.limits.query_timeout)
            .map(|value| value.min(expires_at))
        else {
            return Outcome::Unavailable(Unavailable::Expired);
        };
        match self.published.select(&query, exit) {
            Selection::Published(answer) => {
                let message = answer.response(0);
                Outcome::Answer(Box::new(ReadyAnswer {
                    query,
                    message,
                    cancelled: self.cancelled.child_token(),
                    deadline,
                    route: None,
                }))
            }
            Selection::Unavailable(SelectionUnavailable::DiscoveryRequired) => {
                if !matches!(
                    query.question().query_type(),
                    RecordType::A | RecordType::AAAA
                ) {
                    return Outcome::Unavailable(Unavailable::UnsupportedRecordType);
                }
                let Some(name) = discovery_name(&query) else {
                    return Outcome::Unavailable(Unavailable::InvalidDiscoveryName);
                };
                let ticket = match self.insert_pending(
                    query,
                    name.clone(),
                    PendingKind::Discovery,
                    deadline,
                ) {
                    Ok(ticket) => ticket,
                    Err(reason) => return Outcome::Unavailable(reason),
                };
                Outcome::Discovery(DiscoveryRequest { ticket, name })
            }
            Selection::Unavailable(SelectionUnavailable::UnsupportedRecordType) => {
                Outcome::Unavailable(Unavailable::UnsupportedRecordType)
            }
            Selection::Unavailable(SelectionUnavailable::MissingExitPlan) => {
                Outcome::Unavailable(Unavailable::MissingExitPlan)
            }
            // Candidate configuration never becomes socket/route authority.
            Selection::Resolvers(_)
            | Selection::Unavailable(SelectionUnavailable::MissingResolverPlan) => {
                Outcome::Unavailable(Unavailable::MissingResolverPlan)
            }
            Selection::Unmanaged => Outcome::Unmanaged,
        }
    }

    fn complete(
        &mut self,
        ticket: &Ticket,
        response: NodeDnsQueryResponse,
        now: Instant,
    ) -> Outcome {
        let pending = match self.take_pending(ticket, now) {
            Ok(pending) => pending,
            Err(reason) => return Outcome::Unavailable(reason),
        };
        if pending.kind != PendingKind::Discovery {
            pending.cancelled.cancel();
            return Outcome::Unavailable(Unavailable::InvalidTicket);
        }
        let mut message = pending.query.response();
        let address = match self.binding.validate(&pending, &response) {
            Ok(address) => address,
            Err(Unavailable::NotVisible) => {
                message.metadata.response_code = ResponseCode::Refused;
                None
            }
            Err(reason) => {
                pending.cancelled.cancel();
                return Outcome::Unavailable(reason);
            }
        };
        if let Some(data) = address {
            message.add_answer(Record::from_rdata(
                pending.query.question().name().clone(),
                0,
                data,
            ));
        }
        Outcome::Answer(Box::new(ReadyAnswer {
            query: pending.query,
            message,
            cancelled: pending.cancelled,
            deadline: pending.deadline,
            route: None,
        }))
    }

    fn cancel(&mut self, ticket: &Ticket) -> bool {
        if !Arc::ptr_eq(&self.generation, &ticket.generation) {
            return false;
        }
        ticket.cancelled.cancel();
        self.pending.remove(&ticket.sequence).is_some()
    }

    fn reap(&mut self, now: Instant) {
        self.pending.retain(|_, pending| {
            if now >= pending.deadline {
                pending.cancelled.cancel();
            }
            !pending.cancelled.is_cancelled()
        });
    }

    fn stop(&mut self) {
        self.cancelled.cancel();
        self.pending.clear();
    }

    fn refresh_generation(&mut self) {
        self.stop();
        self.generation = Arc::new(());
        self.cancelled = CancellationToken::new();
        self.sequence = 0;
    }
}

impl Drop for State {
    fn drop(&mut self) {
        self.stop();
    }
}

impl Binding {
    fn validate(
        &self,
        pending: &Pending,
        response: &NodeDnsQueryResponse,
    ) -> Result<Option<RData>, Unavailable> {
        if response.network_id != self.network || response.source_node_id != self.source {
            return Err(Unavailable::BindingMismatch);
        }
        if response.configuration_revision != self.revision {
            return Err(Unavailable::ReloadRequired);
        }
        let record = response.record.as_ref().ok_or(Unavailable::NotVisible)?;
        if record.name != pending.name
            || normalize_dns_name(&record.name).ok().as_ref() != Some(&record.name)
        {
            return Err(Unavailable::BindingMismatch);
        }
        if (record.node_id == self.source) != (record.name == self.self_name)
            || (record.node_id == self.source && record.addresses != self.self_addresses)
            || self
                .peers
                .get(&record.node_id)
                .is_some_and(|addresses| *addresses != record.addresses)
        {
            return Err(Unavailable::BindingMismatch);
        }
        // Partial peers cannot prove absence or identify a target by its address.
        // The Server's exact-name response binds previously undiscovered nodes.
        match pending.query.question().query_type() {
            RecordType::A => Ok(record
                .addresses
                .ipv4
                .map(|address| RData::A(rdata::A(address)))),
            RecordType::AAAA => Ok(record
                .addresses
                .ipv6
                .map(|address| RData::AAAA(rdata::AAAA(address)))),
            _ => Err(Unavailable::UnsupportedRecordType),
        }
    }
}

fn discovery_name(query: &Query) -> Option<String> {
    // DNS wire labels are arbitrary octets. Do not feed escaped presentation
    // strings or lossy UTF-8 into HTTP discovery, or turn a label dot into a
    // separator. A-labels and valid UTF-8 both use the shared normalizer.
    let mut labels = Vec::new();
    for label in query.question().name().iter() {
        let label = std::str::from_utf8(label).ok()?;
        if label
            .chars()
            .any(|ch| matches!(ch, '.' | '\u{3002}' | '\u{ff0e}' | '\u{ff61}'))
        {
            return None;
        }
        labels.push(label);
    }
    normalize_dns_name(&labels.join(".")).ok()
}

#[cfg(test)]
mod tests;
