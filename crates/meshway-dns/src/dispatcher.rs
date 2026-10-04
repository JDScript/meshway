//! Gateways delegate to the host's unique Control driver and approved local
//! business socket adapter. Neither trait passes credentials or bootstrap grants.
use crate::{
    Answer, Begin, Limits, LocalResolverRoute, Owner, Query, ResponseTransport, Ticket,
    TrustedUpstreamResponse, Unavailable,
};
use bytes::Bytes;
use meshway_configuration::DnsTransport;
use meshway_control::ExecutionSnapshot;
use meshway_model::RequestId;
use meshway_protocol::NodeDnsQueryResponse;
use std::{collections::BTreeMap, future::Future, pin::Pin, sync::Arc, time::Instant};
use tokio::sync::{Mutex, watch};

pub type DnsFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("DNS gateway operation failed")]
pub struct GatewayError;

/// Implement using a bounded gateway to the existing unique Control driver.
/// Return only its authenticated typed `Control::query_dns` result. Cancellation
/// must cancel queued work; no cloned credentials or independent HTTP owner.
pub trait DiscoveryGateway: Send + Sync + 'static {
    fn query<'a>(
        &'a self,
        name: &'a str,
    ) -> DnsFuture<'a, Result<NodeDnsQueryResponse, GatewayError>>;
}

/// Exactly one DNS exchange on the sealed route. Payloads exclude TCP framing.
/// Validate actual source endpoint, transaction and frame/payload bounds before
/// accepting bytes. Enforce route validity at every socket I/O poll, including
/// connect/write/read. Prefer future-owned I/O; if a worker accepts the work,
/// retain its route cancellation guard and join it during adapter shutdown.
/// ID randomization is adapter-owned; restore the input ID after validation.
/// No UDP-to-TCP retry is authorized by a UDP route.
pub trait ResolverExchange: Send + Sync + 'static {
    fn exchange<'a>(
        &'a self,
        route: &'a LocalResolverRoute,
        query: Bytes,
    ) -> DnsFuture<'a, Result<Bytes, GatewayError>>;
}

#[derive(Clone)]
pub struct Scope {
    pub snapshot: ExecutionSnapshot,
    /// Selection of demand IDs only. Each endpoint/protocol is checked against
    /// the compiler-produced local_egress binding in this exact snapshot. At most
    /// 256 mappings are accepted (the compiler's current demand budget).
    pub resolver_demands: BTreeMap<String, RequestId>,
}

/// An additional cancellation fence, never a grant or a replacement lease.
pub(crate) struct ScopeFence(watch::Receiver<Option<Scope>>);

impl ScopeFence {
    pub(crate) fn current(&self) -> bool {
        matches!(self.0.has_changed(), Ok(false))
    }
    pub(crate) async fn invalidated(&self) {
        let mut receiver = self.0.clone();
        let _ = receiver.changed().await;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DispatchError {
    #[error("DNS scope is unavailable or changed")]
    Scope,
    #[error("DNS work expired")]
    Expired,
    #[error("DNS upstream or discovery failed")]
    Gateway,
    #[error("DNS result is malformed")]
    Wire,
    #[error("ordinary system resolution must be preserved; no approved adapter is installed")]
    PreserveSystemResolution,
    #[error("DNS plan unavailable: {0:?}")]
    Unavailable(Unavailable),
}

/// A server handler can return only an owner-guarded answer or a typed failure.
/// The service generates its own bounded empty SERVFAIL for failures.
pub trait Dispatcher: Send + Sync + 'static {
    fn dispatch(
        &self,
        query: Query,
        deadline: Instant,
    ) -> DnsFuture<'_, Result<Answer, DispatchError>>;
    fn stop(&self) -> DnsFuture<'_, ()>;
}

struct State {
    scopes: watch::Receiver<Option<Scope>>,
    owner: Option<Owner>,
    stopped: bool,
}

/// One query owner shared by UDP and every TCP connection. No background task,
/// credential, cache, independent lease or socket is owned by this dispatcher.
pub struct OwnerDispatcher {
    state: Mutex<State>,
    discovery: Arc<dyn DiscoveryGateway>,
    exchange: Arc<dyn ResolverExchange>,
    limits: Limits,
    udp_payload_limit: u16,
}

impl OwnerDispatcher {
    pub fn new(
        scopes: watch::Receiver<Option<Scope>>,
        discovery: Arc<dyn DiscoveryGateway>,
        exchange: Arc<dyn ResolverExchange>,
        limits: Limits,
        udp_payload_limit: u16,
    ) -> Result<Self, DispatchError> {
        if limits.max_pending == 0
            || limits.max_pending > 4096
            || limits.query_timeout.is_zero()
            || Instant::now().checked_add(limits.query_timeout).is_none()
            || udp_payload_limit < 512
        {
            return Err(DispatchError::Scope);
        }
        Ok(Self {
            state: Mutex::new(State {
                scopes,
                owner: None,
                stopped: false,
            }),
            discovery,
            exchange,
            limits,
            udp_payload_limit,
        })
    }

    /// Validate/install the latest query-owner scope. This is not proof that all
    /// resolver routes or host DNS resources are installed. Queries also reconcile
    /// automatically; notifications fence old output before this is scheduled.
    pub async fn reconcile(&self) -> Result<(), DispatchError> {
        let mut state = self.state.lock().await;
        self.reconcile_state(&mut state).map(|_| ())
    }

    fn reconcile_state(&self, state: &mut State) -> Result<Scope, DispatchError> {
        if state.stopped {
            return Err(DispatchError::Scope);
        }
        if state.scopes.has_changed().is_err() {
            if let Some(owner) = state.owner.as_mut() {
                owner.stop();
            }
            state.owner = None;
            return Err(DispatchError::Scope);
        }
        // Read the changed flag under the same watch borrow as the value. A
        // send between has_changed() and borrow_and_update() must not mark a
        // replacement seen while retaining the previous query owner.
        let (changed, scope) = {
            let current = state.scopes.borrow_and_update();
            let scope = current
                .as_ref()
                .filter(|scope| scope.resolver_demands.len() <= 256)
                .cloned();
            (current.has_changed(), scope)
        };
        let Some(scope) = scope else {
            if let Some(owner) = state.owner.as_mut() {
                owner.stop();
            }
            state.owner = None;
            return Err(DispatchError::Scope);
        };
        if changed || state.owner.is_none() {
            // Even same-target renewal cancels old tickets. No old operation is
            // completed under a newly extended scope or transplanted generation.
            state.owner = None;
            state.owner = Some(
                Owner::new(scope.snapshot.clone(), self.limits)
                    .map_err(|_| DispatchError::Scope)?,
            );
        }
        scope
            .snapshot
            .ensure_current()
            .map_err(|_| DispatchError::Scope)?;
        Ok(scope)
    }

    async fn execute(&self, query: Query, deadline: Instant) -> Result<Answer, DispatchError> {
        if Instant::now() >= deadline {
            return Err(DispatchError::Expired);
        }
        let (begin, mut scopes, snapshot) = {
            let mut state = self.state.lock().await;
            let scope = self.reconcile_state(&mut state)?;
            if Instant::now() >= deadline {
                return Err(DispatchError::Expired);
            }
            let begin = state
                .owner
                .as_mut()
                .ok_or(DispatchError::Scope)?
                .begin_published_until(query, &scope.resolver_demands, deadline);
            (begin, state.scopes.clone(), scope.snapshot)
        };
        match begin {
            Begin::Answer(answer) => Ok(answer.with_scope_fence(ScopeFence(scopes))),
            Begin::Unavailable(reason) => Err(DispatchError::Unavailable(reason)),
            Begin::Unmanaged => Err(DispatchError::PreserveSystemResolution),
            Begin::DiscoveryRequest(request) => {
                let mut cancellation = CancelOnDrop(Some(request.ticket.clone()));
                let response = tokio::select! { biased;
                    _ = scopes.changed() => return Err(DispatchError::Scope),
                    _ = snapshot.invalidated() => return Err(DispatchError::Scope),
                    _ = request.ticket.cancelled() => return Err(DispatchError::Expired),
                    _ = tokio::time::sleep_until(deadline.into()) => return Err(DispatchError::Expired),
                    result = self.discovery.query(&request.name) => result.map_err(|_| DispatchError::Gateway)?,
                };
                let mut state = self.state.lock().await;
                self.reconcile_state(&mut state)?;
                let result = state
                    .owner
                    .as_mut()
                    .ok_or(DispatchError::Scope)?
                    .complete_discovery(&request.ticket, response);
                let answer = completed(result)?;
                cancellation.0 = None;
                Ok(answer.with_scope_fence(ScopeFence(scopes)))
            }
            Begin::Upstream(work) => {
                let mut cancellation = CancelOnDrop(Some(work.ticket.clone()));
                let query = work
                    .query
                    .encode_upstream(self.udp_payload_limit)
                    .map_err(|_| DispatchError::Wire)?;
                let bytes = tokio::select! { biased;
                    _ = scopes.changed() => return Err(DispatchError::Scope),
                    _ = work.ticket.cancelled() => return Err(DispatchError::Expired),
                    _ = work.route.invalidated() => return Err(DispatchError::Expired),
                    _ = tokio::time::sleep_until(deadline.into()) => return Err(DispatchError::Expired),
                    result = self.exchange.exchange(&work.route, query) => result.map_err(|_| DispatchError::Gateway)?,
                };
                let transport = match work.route.transport() {
                    DnsTransport::Udp => ResponseTransport::Udp,
                    DnsTransport::Tcp => ResponseTransport::Tcp,
                };
                let response = TrustedUpstreamResponse::decode(
                    &work.query,
                    &bytes,
                    transport,
                    self.udp_payload_limit,
                )
                .map_err(|_| DispatchError::Wire)?;
                let mut state = self.state.lock().await;
                self.reconcile_state(&mut state)?;
                let result = state
                    .owner
                    .as_mut()
                    .ok_or(DispatchError::Scope)?
                    .complete_upstream(&work, response);
                let answer = completed(result)?;
                cancellation.0 = None;
                Ok(answer.with_scope_fence(ScopeFence(scopes)))
            }
        }
    }
}

impl Dispatcher for OwnerDispatcher {
    fn dispatch(
        &self,
        query: Query,
        deadline: Instant,
    ) -> DnsFuture<'_, Result<Answer, DispatchError>> {
        Box::pin(self.execute(query, deadline))
    }

    fn stop(&self) -> DnsFuture<'_, ()> {
        Box::pin(async {
            let mut state = self.state.lock().await;
            state.stopped = true;
            if let Some(owner) = state.owner.as_mut() {
                owner.stop();
            }
            state.owner = None;
        })
    }
}

fn completed(begin: Begin) -> Result<Answer, DispatchError> {
    match begin {
        Begin::Answer(answer) => Ok(answer),
        Begin::Unavailable(reason) => Err(DispatchError::Unavailable(reason)),
        _ => Err(DispatchError::Scope),
    }
}

struct CancelOnDrop(Option<Ticket>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if let Some(ticket) = &self.0 {
            ticket.cancel();
        }
    }
}
