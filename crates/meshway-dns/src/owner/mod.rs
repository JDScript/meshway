//! One bounded DNS query owner per installed netmap. MagicDNS answers come
//! from the netmap's complete `meshHosts`; nothing here authorizes resolver
//! traffic beyond the sealed per-query routes.
mod exit;
mod response;
mod system;
use crate::{
    ConfigurationError, ExitSelection, LocalResolverRoute, PublishedDns, Query, ResolverRoute,
    ResponseTransport, Selection, SelectionUnavailable, TrustedUpstreamResponse, WireError,
    encode_response,
};
use bytes::Bytes;
use hickory_proto::{
    op::{Message, ResponseCode},
    rr::RecordType,
};
use meshway_authority::ExecutionSnapshot;
use std::{
    collections::{BTreeMap, VecDeque},
    fmt,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

/// Explicit resource bounds. The timeout bounds work only; it never extends
/// the control snapshot lifetime.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_pending: usize,
    pub query_timeout: Duration,
}

/// Ordinary DNS selection, read from one netmap snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Plan {
    NoExit,
    ViaExit { max_pending: u16, timeout_ms: u32 },
    ServeExitResolver,
    Unavailable,
}

impl Plan {
    pub(crate) fn of(snapshot: &ExecutionSnapshot) -> Option<Self> {
        use meshway_configuration::netmap::DnsPlan;
        let netmap = snapshot.netmap().ok()?;
        Some(match &netmap.netmap.dns.egress {
            DnsPlan::NoExit => Self::NoExit,
            DnsPlan::ViaExit { service } => Self::ViaExit {
                max_pending: service.max_pending,
                timeout_ms: service.timeout_ms,
            },
            DnsPlan::ServeExitResolver { .. } => Self::ServeExitResolver,
            DnsPlan::Unavailable { .. } => Self::Unavailable,
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum OwnerError {
    #[error("DNS limits require nonzero capacity and a representable nonzero timeout")]
    Limits,
    #[error("DNS execution snapshot is not current")]
    Inactive,
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
    UnsupportedRecordType,
    MissingResolverPlan,
    MissingExitPlan,
    InvalidTicket,
    Expired,
    BindingMismatch,
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

    /// Select this and `Owner::invalidated()` around host-driven upstream I/O.
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

/// Work for an exact configured resolver or a bounded ordered system list.
/// Every route is decided by the current netmap. All attempts share the
/// original query ticket.
#[derive(Debug)]
pub struct UpstreamWork {
    pub(crate) ticket: Ticket,
    pub(crate) route: ResolverRoute,
    pub(crate) query: Query,
    pub(crate) fallback: Option<ResolverRoute>,
    pub(crate) remaining: VecDeque<SystemUpstream>,
}

#[derive(Debug)]
pub(crate) struct SystemUpstream {
    pub(crate) route: LocalResolverRoute,
    pub(crate) fallback: Option<LocalResolverRoute>,
}

impl UpstreamWork {
    pub fn ticket(&self) -> &Ticket {
        &self.ticket
    }
    pub fn route(&self) -> &ResolverRoute {
        &self.route
    }
    pub fn query(&self) -> &Query {
        &self.query
    }
}

#[derive(Debug)]
pub enum Begin {
    Answer(Answer),
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
    scope_fence: Option<Box<crate::dispatcher::ScopeFence>>,
}

impl fmt::Debug for Answer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Answer").finish_non_exhaustive()
    }
}

/// One bounded query owner per installed snapshot. No token, HTTP client, ACL,
/// lease renewal, background task, upstream socket or domain grant lives here.
/// Published records use TTL 0: no separate record TTL authority is
/// currently supplied. Pending queries are the only retained query data.
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
        let netmap = snapshot.netmap().map_err(|_| OwnerError::Inactive)?;
        let state = State::from_netmap(&netmap.netmap, limits)?;
        snapshot
            .ensure_current()
            .map_err(|_| OwnerError::Inactive)?;
        Ok(Self { snapshot, state })
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
    /// Cap every ticket, route and answer by the original client's
    /// request deadline. A host may shorten, but never extend, owner authority.
    pub fn begin_published_until(&mut self, query: Query, request_deadline: Instant) -> Begin {
        self.reap();
        if Instant::now() >= request_deadline {
            return Begin::Unavailable(Unavailable::Expired);
        }
        let Some(plan) = Plan::of(&self.snapshot) else {
            return Begin::Unavailable(Unavailable::Inactive);
        };
        let no_exit = matches!(plan, Plan::NoExit | Plan::ServeExitResolver);
        let exit = if no_exit {
            ExitSelection::NoExit
        } else {
            ExitSelection::Exit
        };
        let selected = self.state.published.select(&query, exit);
        if matches!(selected, Selection::ExitResolver) {
            return self.begin_exit(query, request_deadline);
        }
        let Selection::Resolvers(candidates) = selected else {
            let mut begin = self.begin(query, exit);
            if let Begin::Answer(answer) = &mut begin {
                answer.ready.deadline = answer.ready.deadline.min(request_deadline);
            }
            return begin;
        };
        if plan == Plan::Unavailable {
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
        let mut route =
            match LocalResolverRoute::new(self.snapshot.clone(), candidate.key, deadline) {
                Ok(route) => route,
                Err(crate::RouteError::Expired) => return Begin::Unavailable(Unavailable::Expired),
                Err(crate::RouteError::Binding) => {
                    return Begin::Unavailable(Unavailable::MissingResolverPlan);
                }
            };
        let ticket = match self.state.insert_pending(
            query.clone(),
            deadline,
        ) {
            Ok(ticket) => ticket,
            Err(reason) => return Begin::Unavailable(reason),
        };
        route.bind_cancellation(ticket.cancelled.clone());
        Begin::Upstream(Box::new(UpstreamWork {
            ticket,
            route: ResolverRoute::Local(route),
            query,
            fallback: None,
            remaining: VecDeque::new(),
        }))
    }

    pub(crate) fn begin_system(
        &mut self,
        query: Query,
        system: &crate::SystemResolverSnapshot,
        deadline: Instant,
    ) -> Begin {
        self.reap();
        let Some(plan) = Plan::of(&self.snapshot) else {
            return Begin::Unavailable(Unavailable::Inactive);
        };
        // A resolver Node resolves for itself as a no-exit Node does.
        let no_exit = plan == Plan::NoExit || plan == Plan::ServeExitResolver;
        if !no_exit
            || !matches!(
                self.state.published.select(&query, ExitSelection::NoExit),
                Selection::Unmanaged
            )
        {
            return Begin::Unavailable(Unavailable::MissingResolverPlan);
        }
        let deadline = deadline
            .min(self.snapshot.expires_at())
            .min(Instant::now() + self.state.limits.query_timeout);
        // Validate the entire bounded OS list before starting I/O. Missing or
        // expired authority is terminal, never a reason to skip an upstream.
        let mut upstreams = match self.system_upstreams(system, deadline) {
            Ok(upstreams) => upstreams,
            Err(reason) => return Begin::Unavailable(reason),
        };
        let ticket = match self.state.insert_pending(
            query.clone(),
            deadline,
        ) {
            Ok(ticket) => ticket,
            Err(reason) => return Begin::Unavailable(reason),
        };
        for upstream in &mut upstreams {
            upstream.route.bind_cancellation(ticket.cancelled.clone());
            if let Some(fallback) = &mut upstream.fallback {
                fallback.bind_cancellation(ticket.cancelled.clone());
            }
        }
        let first = upstreams.pop_front().expect("nonempty checked system list");
        Begin::Upstream(Box::new(UpstreamWork {
            ticket,
            route: ResolverRoute::Local(first.route),
            query,
            fallback: first.fallback.map(ResolverRoute::Local),
            remaining: upstreams,
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
        if response.query != pending.query
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

    #[cfg(test)]
    pub(crate) fn generation(&self) -> Arc<()> {
        self.state.generation.clone()
    }

    #[cfg(test)]
    pub(crate) fn pending_len(&self) -> usize {
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
            Outcome::Unmanaged => Begin::Unmanaged,
            Outcome::Unavailable(reason) => Begin::Unavailable(reason),
        }
    }
}

struct State {
    published: PublishedDns,
    limits: Limits,
    generation: Arc<()>,
    cancelled: CancellationToken,
    sequence: u64,
    pending: BTreeMap<u64, Pending>,
}

struct Pending {
    query: Query,
    deadline: Instant,
    cancelled: CancellationToken,
}

struct ReadyAnswer {
    query: Query,
    message: Message,
    cancelled: CancellationToken,
    deadline: Instant,
    route: Option<ResolverRoute>,
}

enum Outcome {
    Answer(Box<ReadyAnswer>),
    Unmanaged,
    Unavailable(Unavailable),
}

impl State {
    fn insert_pending(&mut self, query: Query, deadline: Instant) -> Result<Ticket, Unavailable> {
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
                deadline,
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
    fn from_netmap(
        netmap: &meshway_configuration::netmap::NodeNetmap,
        limits: Limits,
    ) -> Result<Self, ConfigurationError> {
        Ok(Self {
            published: PublishedDns::from_netmap(netmap)?,
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
                // The netmap's MagicDNS hosts are the complete visible set:
                // unknown and forbidden names are deliberately
                // indistinguishable (REFUSED, no SOA, no AA, no negative cache).
                let mut message = query.response();
                message.metadata.response_code = ResponseCode::Refused;
                Outcome::Answer(Box::new(ReadyAnswer {
                    query,
                    message,
                    cancelled: self.cancelled.child_token(),
                    deadline,
                    route: None,
                }))
            }
            Selection::Unavailable(SelectionUnavailable::UnsupportedRecordType) => {
                Outcome::Unavailable(Unavailable::UnsupportedRecordType)
            }
            Selection::Unavailable(SelectionUnavailable::MissingExitPlan) => {
                Outcome::Unavailable(Unavailable::MissingExitPlan)
            }
            // Candidate configuration never becomes socket/route authority.
            Selection::Resolvers(_)
            | Selection::ExitResolver
            | Selection::Unavailable(SelectionUnavailable::MissingResolverPlan) => {
                Outcome::Unavailable(Unavailable::MissingResolverPlan)
            }
            Selection::Unmanaged => Outcome::Unmanaged,
        }
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
}

impl Drop for State {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests;
