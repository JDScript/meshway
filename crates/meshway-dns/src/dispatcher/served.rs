//! The resolver Node's side of a netmap `ServeExitResolver` plan: queries from
//! Mesh sources listed in the plan, answered only from this Node's verified OS
//! resolvers. A failed upstream is a failed query; nothing falls back.
//!
//! The source and path arrive in an opaque SecureNode-authenticated request.
//! Socket addresses never substitute for that proof. The original request's
//! guard bounds upstream work and the later encrypted reply.
use super::*;
use meshway_configuration::netmap::DnsPlan;
use meshway_model::NodeId;
use std::collections::BTreeMap;

/// Per-source admitted queries, bounded by each service's `maxPending`.
#[derive(Default)]
pub(super) struct Served(std::sync::Mutex<BTreeMap<NodeId, u16>>);

struct Slot<'a> {
    served: &'a Served,
    source: NodeId,
}
impl Drop for Slot<'_> {
    fn drop(&mut self) {
        if let Ok(mut pending) = self.served.0.lock()
            && let Some(count) = pending.get_mut(&self.source)
        {
            *count -= 1;
            if *count == 0 {
                pending.remove(&self.source);
            }
        }
    }
}

impl Served {
    fn admit(&self, source: NodeId, limit: u16) -> Result<Slot<'_>, DispatchError> {
        let mut pending = self.0.lock().map_err(|_| DispatchError::Scope)?;
        let count = pending.entry(source).or_default();
        if *count >= limit {
            return Err(DispatchError::Unavailable(Unavailable::Capacity));
        }
        *count += 1;
        Ok(Slot {
            served: self,
            source,
        })
    }
}

impl OwnerDispatcher {
    pub async fn dispatch_authenticated(
        &self,
        request: &meshway_dataplane::secure::DnsRequest,
    ) -> Result<Answer, DispatchError> {
        let deadline = request.deadline();
        let query = crate::decode_query(request.bytes().map_err(|_| DispatchError::Expired)?)
            .map_err(|_| DispatchError::Scope)?;
        if Instant::now() >= deadline {
            return Err(DispatchError::Expired);
        }
        let (work, fence, _slot) = {
            let mut state = self.state.lock().await;
            let scope = self.reconcile_state(&mut state)?;
            if scope.snapshot.reference() != request.netmap() {
                return Err(DispatchError::Scope);
            }
            let system = scope
                .system_resolvers
                .clone()
                .ok_or(DispatchError::Unavailable(Unavailable::MissingResolverPlan))?;
            let reading = scope
                .snapshot
                .netmap_reading()
                .map_err(|_| DispatchError::Scope)?;
            let netmap = &reading.netmap.netmap;
            let DnsPlan::ServeExitResolver { services } = &netmap.dns.egress else {
                return Err(DispatchError::Unavailable(Unavailable::MissingExitPlan));
            };
            let service = services
                .iter()
                .find(|service| {
                    service.source == request.source() && service.path.nodes == request.path()
                })
                .ok_or(DispatchError::Scope)?;
            meshway_policy::dns_route::netmap_dns_service(
                netmap,
                service,
                service.source,
                netmap.node_id,
                request.path(),
                request.transport(),
            )
            .map_err(|_| DispatchError::Scope)?;
            let slot = self.served.admit(service.source, service.max_pending)?;
            let deadline = deadline.min(
                Instant::now() + std::time::Duration::from_millis(u64::from(service.timeout_ms)),
            );
            let fence = ScopeFence::new(state.scopes.clone(), scope.clone());
            let begin = state
                .owner
                .as_mut()
                .ok_or(DispatchError::Scope)?
                .begin_served(query, &system, deadline);
            let work = match begin {
                Begin::Upstream(work) => work,
                Begin::Unavailable(reason) => return Err(DispatchError::Unavailable(reason)),
                _ => return Err(DispatchError::Scope),
            };
            (work, fence, slot)
        };
        tokio::select! { biased;
            _ = request.ended() => Err(DispatchError::Expired),
            answer = self.upstream(work, fence) => answer,
        }
    }
}
