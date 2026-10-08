use super::*;

struct AcceptSchedulingGateway {
    inner: Arc<Gateway>,
    prepared: Mutex<Option<meshway_control::connectivity::ValidatedOffer>>,
    accepts: AtomicUsize,
    overtaken: AtomicBool,
    release_peer: Arc<AtomicBool>,
    uncertain: bool,
    invalidate: bool,
}

impl TraversalGateway for AcceptSchedulingGateway {
    fn published(&self, candidate: &CandidateReference) -> bool {
        self.inner.published(candidate)
    }

    fn request(&self, operation: Operation) -> BoxFuture<'_, Result<Reply, Error>> {
        Box::pin(async move {
            use meshway_control::connectivity::{AttemptView, Outcome};
            let prepared = self.prepared.lock().unwrap().clone();
            if matches!(operation, Operation::Query(_) | Operation::Prepare { .. })
                && prepared.is_some()
                && self.accepts.load(Ordering::SeqCst) == 0
            {
                // Deterministically expose the old next-query scheduling bug:
                // no timer or external publication is needed to win this race.
                self.overtaken.store(true, Ordering::SeqCst);
                return std::future::pending().await;
            }
            if matches!(operation, Operation::Query(_)) && prepared.is_some() && self.invalidate {
                // Hold the later read so it cannot mask whether the original
                // slot/guard itself rejects the already-accepted RPC result.
                return std::future::pending().await;
            }
            let local_accept = if let Operation::Accept(offer) = &operation {
                if let Some(original) = &prepared {
                    assert_eq!(offer.offer(), original.offer());
                    assert_eq!(offer.expires_at(), original.expires_at());
                    assert!(offer.ensure_current().is_ok());
                    true
                } else {
                    while !self.release_peer.load(Ordering::SeqCst) {
                        tokio::time::sleep(Duration::from_millis(1)).await;
                    }
                    false
                }
            } else {
                false
            };
            let response = self.inner.request(operation).await?;
            if let Reply::Prepare(Outcome::Current(offer)) = &response {
                let mut prepared = self.prepared.lock().unwrap();
                assert!(
                    prepared.is_none(),
                    "uncertain accept must not create a second offer"
                );
                *prepared = Some(offer.clone());
            }
            if local_accept {
                let index = self.accepts.fetch_add(1, Ordering::SeqCst);
                if !self.release_peer.load(Ordering::SeqCst) {
                    assert!(matches!(
                        response,
                        Reply::Accept(Outcome::Current(AttemptView::Waiting(_)))
                    ));
                }
                if index == 0 && self.uncertain {
                    if self.invalidate {
                        prepared.as_ref().unwrap().cancel();
                    }
                    // The real Server committed this acceptance. Lose only its
                    // reply, keeping the original Control-issued guard/slot.
                    return Err(Error::Unavailable);
                }
            }
            Ok(response)
        })
    }
}

pub(super) async fn accept_scheduling(uncertain: bool, invalidate: bool) {
    let stun = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let stun_address = public(stun.local_addr().unwrap());
    let stop = CancellationToken::new();
    let stopped = stop.clone();
    let service = tokio::spawn(async move {
        let mut bytes = [0; meshway_stun::MAX_DATAGRAM_SIZE];
        loop {
            let (len, source) = tokio::select! {
                _ = stopped.cancelled() => break,
                result = stun.recv_from(&mut bytes) => result.unwrap(),
            };
            let request = meshway_stun::parse_request(&bytes[..len]).unwrap();
            stun.send_to(
                &meshway_stun::encode_response(&request, public(source)),
                source,
            )
            .await
            .unwrap();
        }
    });
    let mut fixture = fixture::Fixture::traversal(stun_address).await;
    let traffic = Arc::new(Traffic::default());
    let factory = Arc::new(NatFactory {
        sockets: Mutex::new(
            [
                fixture.udp.take().unwrap(),
                fixture.reverse_udp.take().unwrap(),
            ]
            .into_iter()
            .map(|socket| (socket.local_addr().unwrap(), socket))
            .collect(),
        ),
        stun: stun_address,
        traffic: traffic.clone(),
    });
    let release_peer = Arc::new(AtomicBool::new(false));
    let mut gateways = Vec::new();
    for mut control in fixture.controls.drain(..2) {
        control.acknowledge_saved().await.unwrap();
        let ticket = control.begin_application().await.unwrap();
        control
            .finish_application(&ticket, Ok(()), chrono::Utc::now())
            .await
            .unwrap();
        gateways.push(Arc::new(AcceptSchedulingGateway {
            inner: Arc::new(Gateway {
                queries: AtomicUsize::new(0),
                control: AsyncMutex::new(control),
                acknowledged: Mutex::new(Vec::new()),
                timeline: None,
            }),
            prepared: Mutex::new(None),
            accepts: AtomicUsize::new(0),
            overtaken: AtomicBool::new(false),
            release_peer: release_peer.clone(),
            uncertain,
            invalidate,
        }));
    }
    let mut managers = Vec::new();
    for (index, gateway) in gateways.iter().enumerate() {
        managers.push(
            Manager::start_with_traversal(
                fixture.identities[index].clone(),
                factory.clone(),
                Default::default(),
                gateway.clone(),
                Sequence::MIN.checked_next().unwrap(),
            )
            .await
            .unwrap(),
        );
    }
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            for (manager, gateway) in managers.iter().zip(&gateways) {
                gateway
                    .inner
                    .report_changes(manager.snapshot().unwrap())
                    .await;
                assert!(
                    !gateway.overtaken.load(Ordering::SeqCst),
                    "prepare/reserve must accept before the next query or peer prepare"
                );
            }
            let expected = if uncertain && !invalidate { 2 } else { 1 };
            if gateways
                .iter()
                .any(|gateway| gateway.accepts.load(Ordering::SeqCst) >= expected)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("immediate local accept, including idempotent uncertainty retry");
    assert!(
        managers
            .iter()
            .all(|manager| manager.snapshot().unwrap().links.is_empty())
    );
    assert!(
        traffic.authorized.lock().unwrap().is_empty(),
        "one accepted participant is not Active"
    );
    let coordinator = gateways
        .iter()
        .position(|gateway| gateway.prepared.lock().unwrap().is_some())
        .unwrap();
    let original = gateways[coordinator]
        .prepared
        .lock()
        .unwrap()
        .clone()
        .unwrap();
    if invalidate {
        assert!(original.ensure_current().is_err());
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(gateways[coordinator].accepts.load(Ordering::SeqCst), 1);
        assert!(traffic.authorized.lock().unwrap().is_empty());
        assert!(
            managers
                .iter()
                .all(|manager| manager.snapshot().unwrap().links.is_empty())
        );
    } else {
        original.ensure_current().unwrap();
        release_peer.store(true, Ordering::SeqCst);
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let left = managers[0].snapshot().unwrap();
                let right = managers[1].snapshot().unwrap();
                if left.links.len() == 1 && right.links.len() == 1 && left.links[0].id == right.links[0].id {
                    for link in [left.links[0].clone(), right.links[0].clone()] {
                        assert!(matches!(link.acquisition, LinkAcquisition::CoordinatedUdp { attempt_id, .. } if attempt_id == original.offer().attempt_id));
                    }
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }).await.expect("same reserved offer completes bilateral authentication");
    }
    for manager in &mut managers {
        manager.shutdown().await.unwrap();
    }
    drop(managers);
    stop.cancel();
    service.await.unwrap();
    for gateway in gateways {
        let gateway = Arc::try_unwrap(gateway).ok().unwrap();
        Arc::try_unwrap(gateway.inner)
            .ok()
            .unwrap()
            .control
            .into_inner()
            .shutdown()
            .await
            .unwrap();
    }
    fixture.close().await;
}
