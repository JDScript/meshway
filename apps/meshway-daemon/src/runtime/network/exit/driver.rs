use super::{Health, Work, scope::PacketGuard};
use meshway_dataplane::exit::{ExitProjection, FlowIncarnation};
use meshway_ipc::Problem;
use meshway_underlay::{CtConfirmation, ExitIncarnation, ExitRole, ExitScope};
use std::collections::BTreeMap;
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;

struct Resource {
    revoked: CancellationToken,
    confirmed: bool,
}

pub(super) async fn run(
    role: ExitRole,
    mut work: mpsc::Receiver<Work>,
    failures: mpsc::Sender<FlowIncarnation>,
    health: watch::Sender<Health>,
    stop: CancellationToken,
) -> Result<(), Problem> {
    let mut resources: BTreeMap<FlowIncarnation, Resource> = BTreeMap::new();
    let mut failed = false;
    while let Some(work) = work.recv().await {
        let mut failure = None;
        match work {
            Work::Barrier(reply) => {
                let _ = reply.send(());
            }
            Work::Projection(event) => match *event {
                ExitProjection::Retire { flow, ack, .. } => {
                    if let Some(resource) = resources.remove(&flow) {
                        resource.revoked.cancel();
                    }
                    let removed = match role.retire_flow(ExitIncarnation::new(flow.bytes())).await {
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
                event => {
                    let (install, flow, tuple, scope, ack) = match event {
                        ExitProjection::Install {
                            flow,
                            tuple,
                            scope,
                            ack,
                        } => (true, flow, tuple, scope, ack),
                        ExitProjection::Renew {
                            flow,
                            tuple,
                            scope,
                            ack,
                        } => (false, flow, tuple, scope, ack),
                        ExitProjection::Retire { .. } => unreachable!("handled above"),
                    };
                    let key = ExitIncarnation::new(flow.bytes());
                    let revoked = resources
                        .get(&flow)
                        .map_or_else(|| stop.child_token(), |r| r.revoked.clone());
                    let deadline = scope
                        .snapshot()
                        .deadline_at(scope.deadline(), scope.deadline_ms());
                    let result = if stop.is_cancelled() || scope.ensure_current().is_err() {
                        false
                    } else if let (Ok(deadline), Some(tuple)) =
                        (deadline, super::scope::tuple(tuple))
                    {
                        let kernel_scope = ExitScope {
                            binding: flow.bytes().into(),
                            revoked: revoked.clone(),
                        };
                        let result = if install {
                            role.install(key, tuple, deadline, kernel_scope).await
                        } else {
                            role.renew(key, deadline, kernel_scope).await
                        };
                        result.is_ok_and(|receipt| receipt.incarnation == key)
                    } else {
                        false
                    };
                    if ack.complete(result).is_ok() {
                        if install {
                            resources.insert(
                                flow,
                                Resource {
                                    revoked,
                                    confirmed: false,
                                },
                            );
                        }
                    } else {
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
                        if !resource.confirmed {
                            match role.confirm(key).await.map_err(|_| ())? {
                                CtConfirmation::Confirmed { incarnation, .. }
                                    if incarnation == key =>
                                {
                                    resource.confirmed = true
                                }
                                _ => return Err(()),
                            }
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
