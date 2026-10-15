use super::coordination::udp_carrier;
use super::socket::socket_compatible;
use super::*;

fn peer_batch(carriers: &BTreeMap<NodeId, Carrier>, after: &mut Option<NodeId>) -> Vec<NodeId> {
    let peers: Vec<_> = carriers
        .keys()
        .filter(|peer| after.is_none_or(|after| **peer > after))
        .chain(
            carriers
                .keys()
                .filter(|peer| after.is_some_and(|after| **peer <= after)),
        )
        .take(8)
        .copied()
        .collect();
    if let Some(last) = peers.last() {
        *after = Some(*last);
    }
    peers
}

pub(super) async fn reserve(
    endpoints: &BTreeMap<[u8; 32], Endpoint>,
    id: [u8; 32],
    offer: &ValidatedOffer,
) -> Result<(), Error> {
    let (reply, receiver) = oneshot::channel();
    endpoints
        .get(&id)
        .ok_or(Error::Unavailable)?
        .commands
        .try_send(Command::Reserve {
            offer: offer.clone(),
            reply,
        })
        .map_err(|_| Error::Capacity)?;
    tokio::select! { biased;
        _ = offer.ended() => Err(Error::Authority),
        result = tokio::time::timeout(Duration::from_secs(1), receiver) => result.map_err(|_| Error::Cancelled)?.map_err(|_| Error::Unavailable)?,
    }
}

pub(super) async fn accept(
    gateway: &Arc<dyn TraversalGateway>,
    endpoints: &BTreeMap<[u8; 32], Endpoint>,
    pending: &mut Pending,
) {
    if pending.active
        || pending.offer.ensure_current().is_err()
        || Instant::now() < pending.accept_at
    {
        return;
    }
    pending.accept_at = Instant::now() + Duration::from_millis(500);
    match gateway
        .request(Operation::Accept(pending.offer.clone()))
        .await
    {
        Ok(Reply::Accept(Outcome::Current(AttemptView::Active(active))))
            if pending.offer.ensure_current().is_ok()
                && active.ensure_current().is_ok()
                && active.validated_offer().offer() == pending.offer.offer() =>
        {
            if let Some(endpoint) = endpoints.get(&pending.endpoint) {
                pending.active = endpoint
                    .commands
                    .try_send(Command::Activate(active))
                    .is_ok();
                if !pending.active {
                    tracing::debug!(attempt = %pending.offer.offer().attempt_id,
                        "Traversal activation not delivered: endpoint command queue full");
                }
            }
        }
        Ok(Reply::Accept(Outcome::Unavailable(_))) => pending.offer.cancel(),
        // Waiting and uncertain RPC outcomes retain the original receiver slot
        // and authority. A later accept retries exactly this offer; neither a
        // new query guard nor prepare alone can authorize activation.
        _ => {}
    }
}

pub(super) async fn prepare(
    shared: &Shared,
    gateway: &Arc<dyn TraversalGateway>,
    endpoints: &BTreeMap<[u8; 32], Endpoint>,
    context: &DiscoveryContext,
    pending: &mut BTreeMap<UdpTraversalId, Pending>,
    coordination: &Coordination,
    after_peer: &mut Option<NodeId>,
) -> Result<(), Error> {
    let plan = &*shared.plan();
    if context.netmap() != plan.reference {
        return Ok(());
    }
    let now = wall_ms();
    let mut carriers = BTreeMap::new();
    for peer in plan.dial_peers() {
        if !shared.wanted(peer) {
            continue;
        }
        // The edge's carrier order; the first admitted Native UDP one.
        for (_, dial) in plan.outgoing(peer)? {
            let carrier = Carrier {
                dialer: plan.node,
                listener: dial.peer,
                listener_key: dial.listener_key.clone(),
                protocol: dial.parameters.transport(),
            };
            if udp_carrier(plan, &carrier) {
                carriers.entry(dial.peer).or_insert(carrier);
                break;
            }
        }
    }
    // Either Node with a need prepares its own dialer carrier; the peer
    // participates on reading the offer, without a need of its own. When both
    // prepare at once the Server admits one live attempt per pair and address
    // family, and the other Node accepts that offer instead.
    carriers.retain(|peer, _| {
        !pending
            .values()
            .any(|pending| pending.offer.peer().candidate.socket.node_id == *peer)
            // Let the selected owner finish its retirement before a new acquisition.
            && !shared.links.lock().is_ok_and(|links| links.contains_key(peer))
            && !coordination.paused(shared, *peer)
    });
    // Advance before the RPC, including errors/no observations. An unavailable
    // prefix must not starve other peers; the cursor survives new content.
    let peers = peer_batch(&carriers, after_peer);
    if peers.is_empty() {
        return Ok(());
    }
    let Reply::Peers(view) = gateway
        .request(Operation::Peers {
            context: context.clone(),
            peers,
        })
        .await?
    else {
        return Err(Error::Configuration);
    };
    if view.state_epoch != context.state_epoch() {
        return Ok(());
    }
    let sockets: Vec<_> = shared
        .sockets
        .lock()
        .map_err(|_| Error::Worker)?
        .iter()
        .map(|(id, fact)| (*id, fact.clone()))
        .collect();
    for (peer, node) in view.nodes {
        if pending.len() >= 8 {
            break;
        }
        if node.freshness != Freshness::Current {
            tracing::debug!(%peer, freshness = ?node.freshness, cause = "peer_not_current", "Traversal preparation has no usable candidate");
            continue;
        }
        let Some(VisibleReport::Peer { report }) = node.report else {
            tracing::debug!(%peer, cause = "peer_report_unavailable", "Traversal preparation has no usable candidate");
            continue;
        };
        let Some(carrier) = carriers.get(&peer) else {
            continue;
        };
        let mut remote_candidates = 0usize;
        let mut compatible_sockets = 0usize;
        let mut local_candidates = 0usize;
        let selected = report
            .endpoints
            .iter()
            .filter(|remote| {
                remote.network == SocketNetwork::Udp
                    && remote.resource
                        == (ResourceRef::Listener {
                            key: carrier.listener_key.clone(),
                        })
                    && remote.candidate.expires_at.timestamp_millis()
                        > i64::try_from(now).unwrap_or(i64::MAX)
            })
            .find_map(|remote| {
                remote_candidates += 1;
                let address =
                    meshway_control::connectivity::udp_recipient(&remote.candidate.address).ok()?;
                sockets.iter().find_map(|(id, fact)| {
                    if fact.bound_address.is_ipv4() != address.is_ipv4()
                        || !socket_compatible(plan, carrier, fact)
                    {
                        return None;
                    }
                    compatible_sockets += 1;
                    fact.endpoints.iter().find_map(|observation| {
                        if observation.expires_at.timestamp_millis() <= i64::try_from(now).ok()? {
                            return None;
                        }
                        let local = fact.candidate(observation).ok()?;
                        local_candidates += 1;
                        gateway
                            .published(&local)
                            .then(|| (*id, local, remote.candidate_reference.clone()))
                    })
                })
            });
        let Some((endpoint, local, remote)) = selected else {
            tracing::debug!(%peer, remote_candidates, compatible_sockets, local_candidates,
                visible_endpoints = report.endpoints.len(), local_sockets = sockets.len(),
                netmap_serial = context.netmap().serial.get(),
                cause = "no_acknowledged_pair", "Traversal preparation has no usable candidate");
            continue;
        };
        let request = prepare_request(context, carrier.clone(), local, remote)
            .map_err(|_| Error::Authority)?;
        // Retry delivery uncertainty with precisely the same request/deadline.
        let operation = || Operation::Prepare {
            context: context.clone(),
            request: Box::new(request.clone()),
        };
        let response = match gateway.request(operation()).await {
            Ok(reply) => Ok(reply),
            Err(_) => gateway.request(operation()).await,
        };
        if let Ok(Reply::Prepare(Outcome::Current(offer))) = response {
            tracing::debug!(%peer, attempt = %offer.offer().attempt_id, "Traversal prepared");
            if reserve(endpoints, endpoint, &offer).await.is_ok() {
                // Install before the RPC so cancellation/uncertainty always
                // retains the exact reserved slot in this content's cleanup set.
                // Accept immediately, before another peer's prepare or query
                // consumes the short bilateral acquisition window.
                let value = pending.entry(offer.offer().attempt_id).or_insert(Pending {
                    offer,
                    endpoint,
                    active: false,
                    accept_at: Instant::now(),
                });
                accept(gateway, endpoints, value).await;
            } else {
                offer.cancel();
                let _ = gateway.request(Operation::Cancel(offer)).await;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_peer_prefix_does_not_starve_later_batches_across_churn() {
        let node = |id| NodeId::new(uuid::Uuid::from_u128(id)).unwrap();
        let mut carriers: BTreeMap<_, _> = (1..=19)
            .map(|id| {
                let peer = node(id);
                (
                    peer,
                    Carrier {
                        dialer: node(100),
                        listener: peer,
                        listener_key: "udp".into(),
                        protocol: TransportProtocol::NativeUdp,
                    },
                )
            })
            .collect();
        let mut after = None;
        let mut queried = std::collections::BTreeSet::new();
        // No peers become pending/connected: model missing State or failed RPCs.
        for _ in 0..3 {
            let batch = peer_batch(&carriers, &mut after);
            assert_eq!(batch.len(), 8);
            assert_eq!(
                batch
                    .iter()
                    .collect::<std::collections::BTreeSet<_>>()
                    .len(),
                8
            );
            queried.extend(batch);
        }
        assert_eq!(queried, carriers.keys().copied().collect());
        // A publication may remove the cursor peer. No reset to the unavailable
        // prefix and no requirement for the cursor's key to remain in the map.
        carriers.remove(&after.unwrap());
        carriers.retain(|peer, _| *peer > node(8));
        queried.clear();
        for _ in 0..2 {
            queried.extend(peer_batch(&carriers, &mut after));
        }
        assert_eq!(queried, carriers.keys().copied().collect());
        carriers.clear();
        assert!(peer_batch(&carriers, &mut after).is_empty());
    }
}
