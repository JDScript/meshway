use super::{Health, Work, scope::PacketGuard};
use meshway_dataplane::exit::{self, ExitProjection, FlowIncarnation};
use meshway_protocol::ipc::Problem;
use meshway_underlay::{CtConfirmation, ExitIncarnation, ExitRole, ExitScope};
use std::collections::{BTreeMap, VecDeque};
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;

struct Resource {
    revoked: CancellationToken,
    confirmed: bool,
    /// A confirmation of this flow's conntrack entry is outstanding.
    confirming: bool,
}

/// One flow's outstanding conntrack confirmation and the scope of the packet
/// that started it; it never holds the work queue.
type Confirmation = (FlowIncarnation, exit::ProjectionScope, bool);

enum Event {
    Work(Work),
    Confirmed(Result<Confirmation, tokio::task::JoinError>),
}

/// At most this many installations share one kernel round.
const ROUND: usize = 64;
/// How long a round waits for the rest of the packet turn that started it
/// when the turn's end ([`Work::Flush`]) has not arrived yet.
const ROUND_WAIT: Duration = Duration::from_millis(5);

struct Installing {
    flow: FlowIncarnation,
    tuple: exit::ExitTuple,
    scope: exit::ProjectionScope,
    ack: exit::ProjectionAck,
}

/// Collect the installations of the packet turn `first` belongs to: they
/// share one kernel round. Packets met meanwhile keep their order after the
/// round (each waits only for its own flow's installation); any other work
/// ends the round and keeps its place.
async fn gather(
    first: Installing,
    work: &mut mpsc::Receiver<Work>,
    deferred: &mut VecDeque<Work>,
) -> Vec<Installing> {
    let mut round = vec![first];
    let deadline = tokio::time::Instant::now() + ROUND_WAIT;
    while round.len() < ROUND && deferred.len() < super::CAPACITY {
        let next = match work.try_recv() {
            Ok(next) => next,
            Err(mpsc::error::TryRecvError::Empty) => {
                match tokio::time::timeout_at(deadline, work.recv()).await {
                    Ok(Some(next)) => next,
                    _ => break,
                }
            }
            Err(mpsc::error::TryRecvError::Disconnected) => break,
        };
        match next {
            Work::Flush => break,
            Work::Projection(event) => match *event {
                ExitProjection::Install {
                    flow,
                    tuple,
                    scope,
                    ack,
                } => round.push(Installing {
                    flow,
                    tuple,
                    scope,
                    ack,
                }),
                other => {
                    deferred.push_back(Work::Projection(Box::new(other)));
                    break;
                }
            },
            packet @ Work::Packet(_) => deferred.push_back(packet),
            other => {
                deferred.push_back(other);
                break;
            }
        }
    }
    round
}

pub(super) async fn run(
    role: ExitRole,
    mut queue: mpsc::Receiver<Work>,
    failures: mpsc::Sender<FlowIncarnation>,
    health: watch::Sender<Health>,
    stop: CancellationToken,
) -> Result<(), Problem> {
    let mut resources: BTreeMap<FlowIncarnation, Resource> = BTreeMap::new();
    let mut failed = false;
    // A forward packet that died at this exit (TTL, MTU) never confirms; its
    // up to 3 s wait runs beside every other flow's work, not before it.
    let mut confirming = tokio::task::JoinSet::<Confirmation>::new();
    // Work taken out of the queue by a round, in its original order.
    let mut deferred = VecDeque::new();
    loop {
        let work = match deferred.pop_front() {
            Some(work) => Event::Work(work),
            None => tokio::select! {
                Some(done) = confirming.join_next(), if !confirming.is_empty() => Event::Confirmed(done),
                work = queue.recv() => match work {
                    Some(work) => Event::Work(work),
                    None => break,
                },
            },
        };
        let mut failure = None;
        let work = match work {
            Event::Work(work) => Some(work),
            Event::Confirmed(Ok((flow, original, confirmed))) => {
                if let Some(resource) = resources.get_mut(&flow) {
                    resource.confirming = false;
                    resource.confirmed |= confirmed;
                    if !confirmed && original.ensure_current().is_ok() {
                        resource.revoked.cancel();
                        failure = Some(flow);
                    }
                }
                None
            }
            Event::Confirmed(Err(_)) => {
                failed = true;
                None
            }
        };
        if let Some(work) = work {
            match work {
                Work::Barrier(reply) => {
                    let _ = reply.send(());
                }
                Work::Flush => {}
                Work::Projection(event) => match *event {
                    ExitProjection::Retire { flow, ack, .. } => {
                        if let Some(resource) = resources.remove(&flow) {
                            resource.revoked.cancel();
                        }
                        let removed =
                            match role.retire_flow(ExitIncarnation::new(flow.bytes())).await {
                                Ok(()) => true,
                                // The Host may already have retired the whole role on
                                // lease loss. Its verified all-flow cleanup also proves
                                // this exact projection absent; an error never does.
                                Err(_) if !role.is_active() => role.retire().await.is_ok(),
                                Err(_) => false,
                            };
                        ack.complete(removed);
                        failed |= !removed;
                    }
                    ExitProjection::Install {
                        flow,
                        tuple,
                        scope,
                        ack,
                    } => {
                        let first = Installing {
                            flow,
                            tuple,
                            scope,
                            ack,
                        };
                        let round = gather(first, &mut queue, &mut deferred).await;
                        let mut submitted = Vec::new();
                        let mut projections = Vec::new();
                        let mut outcomes = Vec::new();
                        for item in round {
                            let key = ExitIncarnation::new(item.flow.bytes());
                            let revoked = resources
                                .get(&item.flow)
                                .map_or_else(|| stop.child_token(), |r| r.revoked.clone());
                            let ready = !stop.is_cancelled() && item.scope.ensure_current().is_ok();
                            match (ready, super::scope::tuple(item.tuple.clone())) {
                                (true, Some(tuple)) => {
                                    projections.push((
                                        key,
                                        tuple,
                                        meshway_underlay::ExitCutoff {
                                            at: item.scope.deadline(),
                                            ms: item.scope.deadline_ms(),
                                        },
                                        ExitScope {
                                            binding: item.flow.bytes().into(),
                                            revoked: revoked.clone(),
                                        },
                                    ));
                                    submitted.push(outcomes.len());
                                    outcomes.push((item, revoked, false));
                                }
                                _ => outcomes.push((item, revoked, false)),
                            }
                        }
                        let results = role.install_batch(projections).await;
                        for (index, result) in submitted.into_iter().zip(results) {
                            let key = ExitIncarnation::new(outcomes[index].0.flow.bytes());
                            outcomes[index].2 =
                                result.is_ok_and(|receipt| receipt.incarnation == key);
                        }
                        // Each flow's own acknowledgement, whatever the others'.
                        for (item, revoked, installed) in outcomes {
                            if item.ack.complete(installed).is_ok() {
                                resources.insert(
                                    item.flow,
                                    Resource {
                                        revoked,
                                        confirmed: false,
                                        confirming: false,
                                    },
                                );
                            } else {
                                revoked.cancel();
                                if failures.try_send(item.flow).is_err() {
                                    failed = true;
                                }
                            }
                        }
                    }
                    ExitProjection::Renew {
                        flow,
                        tuple,
                        scope,
                        ack,
                    } => {
                        let key = ExitIncarnation::new(flow.bytes());
                        let revoked = resources
                            .get(&flow)
                            .map_or_else(|| stop.child_token(), |r| r.revoked.clone());
                        let result = if stop.is_cancelled()
                            || scope.ensure_current().is_err()
                            || super::scope::tuple(tuple).is_none()
                        {
                            false
                        } else {
                            let deadline = meshway_underlay::ExitCutoff {
                                at: scope.deadline(),
                                ms: scope.deadline_ms(),
                            };
                            let kernel_scope = ExitScope {
                                binding: flow.bytes().into(),
                                revoked: revoked.clone(),
                            };
                            role.renew(key, deadline, kernel_scope)
                                .await
                                .is_ok_and(|receipt| receipt.incarnation == key)
                        };
                        if ack.complete(result).is_err() {
                            revoked.cancel();
                            failure = Some(flow);
                        }
                    }
                },
                Work::Packet(packet) => {
                    let packet = *packet;
                    if let Some(flow) = packet.exit_flow()
                        && let Some(resource) = resources.get_mut(&flow)
                        && !stop.is_cancelled()
                    {
                        let key = ExitIncarnation::new(flow.bytes());
                        if packet.projection_ready().await.is_err() {
                            continue;
                        }
                        let Ok(scope) = packet.exit_scope() else {
                            continue;
                        };
                        let original = scope.clone();
                        let sent = async {
                        let bytes = packet
                            .release(|action| match action {
                                meshway_dataplane::Action::Exit { packet, .. } => {
                                    Some(packet.as_bytes().to_vec())
                                }
                                _ => None,
                            })
                            .map_err(|_| ())?
                            .ok_or(())?;
                        role.send(
                            key,
                            bytes,
                            PacketGuard {
                                scope,
                                stop: stop.clone(),
                            },
                        )
                        .await
                        .map_err(|_| ())?;
                        if !resource.confirmed && !resource.confirming {
                            resource.confirming = true;
                            let role = role.clone();
                            let original = original.clone();
                            confirming.spawn(async move {
                                let confirmed = matches!(
                                    role.confirm(key).await,
                                    Ok(CtConfirmation::Confirmed { incarnation, .. }) if incarnation == key
                                );
                                (flow, original, confirmed)
                            });
                        }
                        Ok::<_, ()>(())
                    }
                    .await;
                        if sent.is_err() && original.ensure_current().is_ok() {
                            resource.revoked.cancel();
                            failure = Some(flow);
                        }
                    }
                }
            }
        }
        if let Some(flow) = failure
            && failures.try_send(flow).is_err()
        {
            failed = true;
        }
        if failed {
            stop.cancel();
        }
        let next = Health {
            flows: resources
                .values()
                .filter(|resource| !resource.revoked.is_cancelled())
                .count() as u32,
            failed,
        };
        health.send_if_modified(|current| {
            if *current == next {
                return false;
            }
            *current = next;
            true
        });
    }
    stop.cancel();
    confirming.abort_all();
    while confirming.join_next().await.is_some() {}
    let retired = role.retire().await.map_err(|_| Problem::Underlay);
    health.send_replace(Health {
        flows: 0,
        failed: failed || retired.is_err(),
    });
    retired.and(if failed {
        Err(Problem::Underlay)
    } else {
        Ok(())
    })
}
