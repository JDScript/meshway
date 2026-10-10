//! DNS-purpose exchanges scheduled by the existing packet owner. Local
//! resolver sockets stay with Underlay; no Control HTTP call participates.
use meshway_connectivity::AdjacencyInterest;
use meshway_dataplane::secure::{CircuitGrant, DnsReply, DnsRequest, SecureNode};
use meshway_dns::{
    Bytes, DnsFuture, GatewayError, LocalResolverRoute, MeshResolverRoute, OwnerDispatcher,
    ResolverExchange,
};
use meshway_model::EventId;
use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, oneshot};

pub(in crate::runtime::network) struct Query {
    route: MeshResolverRoute,
    bytes: Option<Bytes>,
    reply: oneshot::Sender<Result<Bytes, GatewayError>>,
}

pub(super) struct Exchange {
    local: Arc<dyn ResolverExchange>,
    queries: mpsc::Sender<Query>,
}
impl Exchange {
    pub(super) fn new(local: Arc<dyn ResolverExchange>, queries: mpsc::Sender<Query>) -> Arc<Self> {
        Arc::new(Self { local, queries })
    }
}
impl ResolverExchange for Exchange {
    fn exchange<'a>(
        &'a self,
        route: &'a LocalResolverRoute,
        query: Bytes,
    ) -> DnsFuture<'a, Result<Bytes, GatewayError>> {
        self.local.exchange(route, query)
    }
    fn validate_mesh(&self, route: &MeshResolverRoute) -> Result<(), GatewayError> {
        route.ensure_current().map_err(|_| GatewayError)?;
        if self.queries.is_closed() {
            return Err(GatewayError);
        }
        Ok(())
    }
    fn exchange_mesh<'a>(
        &'a self,
        route: &'a MeshResolverRoute,
        query: Bytes,
    ) -> DnsFuture<'a, Result<Bytes, GatewayError>> {
        Box::pin(async move {
            self.validate_mesh(route)?;
            if query.len() > meshway_dataplane::secure::MAX_DNS_BYTES {
                return Err(GatewayError);
            }
            let (reply, response) = oneshot::channel();
            let operation = async {
                self.queries
                    .send(Query {
                        route: route.clone(),
                        bytes: Some(query),
                        reply,
                    })
                    .await
                    .map_err(|_| GatewayError)?;
                response.await.map_err(|_| GatewayError)?
            };
            let bytes = tokio::select! { biased;
                _ = route.ended() => return Err(GatewayError),
                _ = tokio::time::sleep_until(route.deadline().into()) => return Err(GatewayError),
                result = operation => result?,
            };
            route.ensure_current().map_err(|_| GatewayError)?;
            Ok(bytes)
        })
    }
}

type Served = (DnsRequest, Result<Bytes, GatewayError>);

#[derive(Default)]
pub(in crate::runtime::network) struct Queries {
    pending: BTreeMap<EventId, Query>,
    circuit: Option<[u8; 32]>,
    serving: tokio::task::JoinSet<Served>,
}
impl Queries {
    pub fn enqueue(&mut self, query: Query) {
        if self.pending.len() >= 128
            || query.reply.is_closed()
            || query.route.ensure_current().is_err()
        {
            let _ = query.reply.send(Err(GatewayError));
            return;
        }
        let mut bytes = [0; 16];
        let id = getrandom::fill(&mut bytes)
            .ok()
            .and_then(|()| EventId::new(uuid::Builder::from_random_bytes(bytes).into_uuid()).ok());
        let Some(id) = id.filter(|id| !self.pending.contains_key(id)) else {
            let _ = query.reply.send(Err(GatewayError));
            return;
        };
        self.pending.insert(id, query);
    }

    pub fn receive(&mut self, request: DnsRequest, dispatcher: Option<Arc<OwnerDispatcher>>) {
        let Some(dispatcher) = dispatcher.filter(|_| self.serving.len() < 128) else {
            return;
        };
        self.serving.spawn(async move {
            let result = match dispatcher.dispatch_authenticated(&request).await {
                Ok(answer) => answer
                    .encode(meshway_dns::ResponseTransport::Tcp, 1232)
                    .map_err(|_| GatewayError),
                Err(_) => Err(GatewayError),
            };
            (request, result)
        });
    }

    pub fn reply(&mut self, reply: DnsReply) {
        let Some(query) = self.pending.remove(&reply.id()) else {
            return;
        };
        let result = query
            .route
            .ensure_current()
            .map_err(|_| GatewayError)
            .and_then(|()| reply.release(Bytes::from).map_err(|_| GatewayError));
        let _ = query.reply.send(result);
    }

    pub fn wake_at(&mut self) -> Option<tokio::time::Instant> {
        self.pending
            .retain(|_, query| !query.reply.is_closed() && query.route.ensure_current().is_ok());
        (!self.pending.is_empty() || !self.serving.is_empty())
            .then(|| tokio::time::Instant::now() + Duration::from_millis(50))
    }

    pub async fn drive(
        &mut self,
        secure: &mut SecureNode,
        desired: &meshway_engine::Desired,
        manager: &meshway_connectivity::Manager,
    ) {
        self.pending.retain(|_, query| {
            !query.reply.is_closed()
                && query.route.ensure_current().is_ok()
                && query.route.netmap() == desired.reference()
        });
        for _ in 0..32 {
            let Some(served) = self.serving.try_join_next() else {
                break;
            };
            if let Ok((request, Ok(bytes))) = served
                && let Ok(frame) = secure.answer_dns(request, &bytes)
            {
                let _ = manager.send(frame).await;
            }
        }
        let Some(query) = self.pending.values().find(|query| query.bytes.is_some()) else {
            return;
        };
        let service = query.route.service().clone();
        let grant = CircuitGrant {
            purpose: meshway_dataplane::Purpose::Dns,
            target: meshway_dataplane::Target::Mesh { node: service.exit },
        };
        let known = self
            .circuit
            .and_then(|id| secure.circuit_info(id).ok().flatten())
            .filter(|circuit| circuit.grant == grant && circuit.path == service.path.nodes);
        let ready = match known {
            Some(info) if info.state == meshway_dataplane::secure::CircuitState::Established => {
                Some(info.id)
            }
            Some(info) if info.state != meshway_dataplane::secure::CircuitState::Closed => {
                if secure
                    .handshake_retry_at(info.id)
                    .ok()
                    .flatten()
                    .is_some_and(|at| at <= Instant::now())
                    && let Ok(Some(frame)) = secure.retry_handshake(info.id)
                {
                    let _ = manager.send(frame).await;
                }
                None
            }
            _ => {
                self.circuit = None;
                let Some(peer) = service.path.nodes.get(1).copied() else {
                    return;
                };
                manager.interest().interest(peer);
                let Some(carrier) = manager
                    .carriers()
                    .ok()
                    .and_then(|mut carriers| carriers.remove(&peer))
                else {
                    return;
                };
                if let Ok(frame) = secure.start_grant(grant, service.path.nodes.clone(), &carrier) {
                    self.circuit = Some(frame.circuit());
                    let _ = manager.send(frame).await;
                }
                None
            }
        };
        let Some(circuit) = ready else {
            return;
        };
        for (id, query) in self
            .pending
            .iter_mut()
            .filter(|(_, query)| query.bytes.is_some())
            .take(32)
        {
            let bytes = query.bytes.take().expect("unsent query");
            if query.reply.is_closed() || query.route.ensure_current().is_err() {
                continue;
            }
            if let Ok(frame) = secure.query_dns(
                circuit,
                *id,
                query.route.transport(),
                &bytes,
                query.route.deadline(),
            ) {
                let _ = manager.send(frame).await;
            }
        }
    }

    pub async fn stop(&mut self) {
        self.pending.clear();
        self.circuit = None;
        self.serving.abort_all();
        while self.serving.join_next().await.is_some() {}
    }
}
