//! The bounded adapter to the existing Control owner. A dropped caller closes
//! its reply and cancels queued/in-flight HTTP, never another endpoint receiver.
use meshway_connectivity::{
    Error,
    traversal::{Operation, Reply, TraversalGateway},
};
use meshway_model::{NodeId, Sequence, state::NodeState};
use meshway_protocol::{
    InstanceRequest,
    connectivity::{CandidateReference, SocketReference},
};
use std::{sync::Arc, time::Duration};
use tokio::{
    sync::{mpsc, oneshot, watch},
    time::Instant,
};

#[derive(Clone, Default)]
pub(super) struct Acknowledged {
    pub sequence: Option<Sequence>,
    pub candidates: Vec<CandidateReference>,
}

pub(in crate::runtime) struct Request {
    operation: Operation,
    enqueued: Instant,
    deadline: Instant,
    reply: oneshot::Sender<Result<Reply, Error>>,
}

fn operation_name(operation: &Operation) -> &'static str {
    match operation {
        Operation::Query(_) => "query",
        Operation::Peers { .. } => "peers",
        Operation::Prepare { .. } => "prepare",
        Operation::Accept(_) => "accept",
        Operation::Cancel(_) => "cancel",
    }
}
pub(super) struct Gateway {
    requests: mpsc::Sender<super::Request>,
    acknowledged: watch::Receiver<Acknowledged>,
}
impl Gateway {
    pub fn new(
        requests: mpsc::Sender<super::Request>,
        acknowledged: watch::Receiver<Acknowledged>,
    ) -> Arc<Self> {
        Arc::new(Self {
            requests,
            acknowledged,
        })
    }
}
impl TraversalGateway for Gateway {
    fn request(
        &self,
        operation: Operation,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Reply, Error>> + Send + '_>>
    {
        Box::pin(async move {
            let expires = match &operation {
                Operation::Query(snapshot) => snapshot.expires_at(),
                Operation::Peers { context, .. } | Operation::Prepare { context, .. } => {
                    context.expires_at()
                }
                Operation::Accept(offer) => offer.expires_at(),
                Operation::Cancel(offer) => {
                    offer.cancel();
                    offer.expires_at()
                }
            };
            let enqueued = Instant::now();
            let deadline = (enqueued + Duration::from_secs(2)).min(expires.into());
            if Instant::now() >= deadline {
                return Err(Error::Authority);
            }
            let (reply, received) = oneshot::channel();
            self.requests
                .try_send(super::Request::Traversal(Request {
                    operation,
                    enqueued,
                    deadline,
                    reply,
                }))
                .map_err(|_| Error::Capacity)?;
            tokio::time::timeout_at(deadline, received)
                .await
                .map_err(|_| Error::Cancelled)?
                .map_err(|_| Error::Unavailable)?
        })
    }
    fn published(&self, candidate: &CandidateReference) -> bool {
        let acknowledged = self.acknowledged.borrow();
        acknowledged.sequence.is_some() && acknowledged.candidates.contains(candidate)
    }
}

pub(super) async fn dispatch(
    control: &mut meshway_control::Control,
    request: Request,
) -> Result<(), meshway_control::Error> {
    let Request {
        operation,
        enqueued,
        deadline,
        mut reply,
    } = request;
    if reply.is_closed() || Instant::now() >= deadline {
        tracing::debug!(
            operation = operation_name(&operation),
            cause = "queue_expired",
            queue_ms = enqueued.elapsed().as_millis() as u64,
            "Traversal request expired"
        );
        return Ok(());
    }
    let name = operation_name(&operation);
    let started = Instant::now();
    tracing::debug!(
        operation = name,
        queue_ms = started.duration_since(enqueued).as_millis() as u64,
        remaining_ms = deadline.saturating_duration_since(started).as_millis() as u64,
        "Traversal request dispatched"
    );
    let operation = async {
        Ok(match operation {
            Operation::Query(snapshot) => {
                Reply::Query(control.query_udp_traversal(&snapshot).await?)
            }
            Operation::Peers { context, peers } => {
                Reply::Peers(control.query_connectivity_peers(&context, &peers).await?)
            }
            Operation::Prepare { context, request } => {
                Reply::Prepare(control.prepare_udp_traversal(&context, &request).await?)
            }
            Operation::Accept(offer) => Reply::Accept(control.accept_udp_traversal(&offer).await?),
            Operation::Cancel(offer) => {
                control.cancel_udp_traversal(&offer).await?;
                Reply::Cancelled
            }
        })
    };
    let result: Result<Reply, meshway_control::Error> = tokio::select! { biased;
        _ = reply.closed() => {
            tracing::debug!(operation = name, outcome = "caller_cancelled", rpc_ms = started.elapsed().as_millis() as u64, "Traversal request completed");
            return Ok(());
        },
        result = tokio::time::timeout_at(deadline, operation) => match result {
            Ok(result) => result,
            Err(_) => { tracing::debug!(operation = name, cause = "http_deadline", rpc_ms = started.elapsed().as_millis() as u64, "Traversal request expired"); let _ = reply.send(Err(Error::Cancelled)); return Ok(()); }
        },
    };
    tracing::debug!(
        operation = name,
        outcome = if result.is_ok() {
            "ok"
        } else {
            "control_error"
        },
        rpc_ms = started.elapsed().as_millis() as u64,
        "Traversal request completed"
    );
    match result {
        Ok(result) => {
            use meshway_control::connectivity::{AttemptView, Outcome};
            let scope_changed = matches!(
                &result,
                Reply::Query(Outcome::Unavailable(
                    meshway_protocol::connectivity::UdpTraversalUnavailableReason::ScopeChanged
                ))
            );
            match &result {
                Reply::Query(Outcome::Unavailable(reason)) => tracing::debug!(
                    operation = name,
                    ?reason,
                    "Traversal metadata unavailable response"
                ),
                Reply::Prepare(Outcome::Current(offer)) => {
                    let value = offer.offer();
                    tracing::debug!(operation = name, attempt = %value.attempt_id,
                        local = %offer.local().candidate.socket.node_id,
                        peer = %offer.peer().candidate.socket.node_id,
                        source_instance = %offer.local().candidate.socket.instance.instance_id,
                        netmap_serial = offer.snapshot().reference().serial.get(),
                        peer_netmap_serial = offer.peer().candidate.socket.netmap.serial.get(),
                        expires_at_ms = value.expires_at_ms, "Traversal offer returned");
                }
                Reply::Accept(Outcome::Current(view)) => {
                    let (offer, phase) = match view {
                        AttemptView::Waiting(offer) => (offer, "waiting"),
                        AttemptView::Active(active) => (active.validated_offer(), "active"),
                    };
                    tracing::debug!(operation = name, attempt = %offer.offer().attempt_id,
                        local = %offer.local().candidate.socket.node_id,
                        peer = %offer.peer().candidate.socket.node_id,
                        netmap_serial = offer.snapshot().reference().serial.get(),
                        expires_at_ms = offer.expires_at_ms(), phase, "Traversal acceptance returned");
                }
                Reply::Prepare(Outcome::Unavailable(reason))
                | Reply::Accept(Outcome::Unavailable(reason)) => {
                    tracing::debug!(operation = name, ?reason, "Traversal unavailable response")
                }
                _ => {}
            }
            let _ = reply.send(Ok(result));
            if scope_changed {
                Err(meshway_control::Error::StateScopeChanged)
            } else {
                Ok(())
            }
        }
        Err(error) => {
            let _ = reply.send(Err(Error::Unavailable));
            Err(error)
        }
    }
}

pub(super) fn candidates(
    state: &NodeState,
    node_id: NodeId,
    instance: &InstanceRequest,
) -> Vec<CandidateReference> {
    state
        .sockets
        .iter()
        .flat_map(|(id, socket)| {
            let netmap = socket.netmap.or(state.configuration.applied);
            socket.endpoints.iter().filter_map(move |observation| {
                let reference = SocketReference {
                    node_id,
                    instance: instance.clone(),
                    netmap: netmap?,
                    socket_id: id.clone(),
                    socket_generation: socket.generation,
                    resource: socket.resource.clone(),
                };
                CandidateReference::from_observation(
                    reference,
                    socket.network,
                    socket.bound_address,
                    observation,
                )
                .ok()
            })
        })
        .take(256)
        .collect()
}
