//! The resolver Node's side of a netmap `ServeExitResolver` plan: queries from
//! Mesh sources listed in the plan, answered only from this Node's verified OS
//! resolvers (or, under `dns.overrideLocal`, its configured default
//! resolvers). A failed upstream is a failed query; nothing falls back.
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
    /// The reply to one authenticated request: its answer encoded for the
    /// request's own transport (UDP truncates within the payload limit and
    /// the source retries on its separately permitted TCP), or SERVFAIL when
    /// it cannot be served. `None` once the request itself has ended.
    pub async fn serve_authenticated(
        &self,
        request: &meshway_dataplane::secure::DnsRequest,
    ) -> Option<Bytes> {
        let transport = match request.transport() {
            meshway_configuration::DnsTransport::Udp => crate::ResponseTransport::Udp,
            meshway_configuration::DnsTransport::Tcp => crate::ResponseTransport::Tcp,
        };
        let answer = self.dispatch_authenticated(request).await;
        if let Err(error) = &answer {
            tracing::debug!(?error, "exit resolver query not served");
        }
        if let Ok(answer) = &answer
            && let Ok(bytes) = answer.encode(transport, self.udp_payload_limit)
            && bytes.len() <= meshway_dataplane::secure::MAX_DNS_BYTES
        {
            return Some(bytes);
        }
        let query = crate::decode_query(request.bytes().ok()?).ok()?;
        let mut message = query.response();
        message.metadata.response_code = hickory_proto::op::ResponseCode::ServFail;
        crate::encode_response(&query, &message, transport, self.udp_payload_limit).ok()
    }

    pub(crate) async fn dispatch_authenticated(
        &self,
        request: &meshway_dataplane::secure::DnsRequest,
    ) -> Result<Answer, DispatchError> {
        let deadline = request.deadline();
        let query = crate::decode_query(request.bytes().map_err(|_| DispatchError::Expired)?)
            .map_err(|_| DispatchError::Scope)?;
        if Instant::now() >= deadline {
            return Err(DispatchError::Expired);
        }
        let (work, _slot, desired, service) = {
            let mut state = self.state.lock().await;
            let scope = self.reconcile_state(&mut state)?;
            if scope.desired.reference() != request.netmap() {
                return Err(DispatchError::Scope);
            }
            let served = scope.served_resolvers.clone();
            let netmap = scope.desired.netmap();
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
            let begin = state
                .owner
                .as_mut()
                .ok_or(DispatchError::Scope)?
                .begin_served(query, served.as_ref(), deadline);
            let work = match begin {
                Begin::Upstream(work) => work,
                Begin::Unavailable(reason) => return Err(DispatchError::Unavailable(reason)),
                _ => return Err(DispatchError::Scope),
            };
            (work, slot, scope.desired.clone(), service.clone())
        };
        let answer = tokio::select! { biased;
            _ = request.ended() => Err(DispatchError::Expired),
            answer = self.upstream(work) => answer,
        }?;
        self.report_association(&desired, &service, &answer);
        Ok(answer)
    }

    /// Complete the served plan's association task for an answer whose name
    /// a name rule of the source decides, unless an association covering it
    /// keeps at least half of the life this answer would give.
    fn report_association(
        &self,
        desired: &meshway_engine::Desired,
        service: &meshway_configuration::netmap::DnsService,
        answer: &Answer,
    ) {
        let Some(reports) = &self.reports else {
            return;
        };
        // An unassociable answer gets no completion; its source refuses it.
        let crate::association::Facts::Associable(facts) =
            crate::association::answered(answer.query(), answer.message())
        else {
            return;
        };
        if !desired
            .compiler()
            .egress_rule_for_name(service.source, &facts.name)
            .is_ok_and(|hit| hit.is_some())
        {
            return;
        }
        let now = crate::association::wall_ms();
        if desired
            .associations()
            .covering(
                service.source,
                &facts.name,
                facts.record_type,
                &facts.addresses,
            )
            .is_some_and(|known| known.expires_at_ms >= now + facts.lifetime_ms() / 2)
        {
            return;
        }
        match crate::association::report(desired.reference(), service, &facts) {
            Ok(report) => {
                if reports.try_send(report).is_err() {
                    tracing::debug!("Association report queue full; completion dropped");
                }
            }
            Err(_) => tracing::debug!("Association report identifier unavailable"),
        }
    }
}
