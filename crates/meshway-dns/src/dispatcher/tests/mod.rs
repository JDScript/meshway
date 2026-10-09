//! A temporary real Server/Control owns every execution grant. Only the
//! endpoint exchange is scripted; no host DNS or kernel installation is used.
mod support;
use super::*;
use crate::{SystemResolverSnapshot, decode_query};
use hickory_proto::{
    op::{Message, MessageType, OpCode, Query as Question},
    rr::{Name, RecordType},
};
use meshway_configuration::execution::{BusinessProtocol, BusinessTarget, ExecutionAddress};
use meshway_control::Control;
use meshway_protocol::{DemandRequest, InstanceRequest};
use std::{collections::VecDeque, net::SocketAddr, sync::Mutex as StdMutex, time::Duration};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

struct NoDiscovery;
impl DiscoveryGateway for NoDiscovery {
    fn query<'a>(
        &'a self,
        _: &'a str,
    ) -> DnsFuture<'a, Result<NodeDnsQueryResponse, GatewayError>> {
        Box::pin(async { panic!("ordinary system query entered discovery") })
    }
}

#[derive(Clone, Copy)]
enum Step {
    Hang,
    Fail,
    Truncated,
    Answer,
}
struct Exchange {
    steps: StdMutex<VecDeque<Step>>,
    calls: StdMutex<Vec<LocalResolverRoute>>,
    started: Notify,
}
impl Exchange {
    fn new(steps: &[Step]) -> Arc<Self> {
        Arc::new(Self {
            steps: StdMutex::new(steps.iter().copied().collect()),
            calls: StdMutex::new(Vec::new()),
            started: Notify::new(),
        })
    }
    async fn entered(&self, count: usize) {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let wake = self.started.notified();
                if self.calls.lock().unwrap().len() >= count {
                    return;
                }
                wake.await;
            }
        })
        .await
        .unwrap();
    }
    fn endpoints(&self) -> Vec<(SocketAddr, DnsTransport)> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .map(|r| (r.endpoint(), r.transport()))
            .collect()
    }
}
impl ResolverExchange for Exchange {
    fn exchange<'a>(
        &'a self,
        route: &'a LocalResolverRoute,
        bytes: Bytes,
    ) -> DnsFuture<'a, Result<Bytes, GatewayError>> {
        Box::pin(async move {
            route.ensure_current().map_err(|_| GatewayError)?;
            let step = self
                .steps
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected extra exchange");
            self.calls.lock().unwrap().push(route.clone());
            self.started.notify_one();
            match step {
                Step::Hang => std::future::pending().await,
                Step::Fail => Err(GatewayError),
                Step::Truncated | Step::Answer => {
                    let mut response = decode_query(&bytes).unwrap().response();
                    response.metadata.truncation = matches!(step, Step::Truncated);
                    Ok(Bytes::from(response.to_vec().unwrap()))
                }
            }
        })
    }
}

fn query() -> Query {
    let mut message = Message::new(123, MessageType::Query, OpCode::Query);
    message.add_query(Question::query(
        Name::from_ascii("ordinary.dispatcher.test.").unwrap(),
        RecordType::A,
    ));
    decode_query(&message.to_vec().unwrap()).unwrap()
}

fn request_id() -> RequestId {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).unwrap();
    bytes[6] = (bytes[6] & 15) | 0x40;
    bytes[8] = (bytes[8] & 63) | 0x80;
    let hex = format!("{:032x}", u128::from_be_bytes(bytes));
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
    .parse()
    .unwrap()
}

async fn fixture(control: &mut Control) -> Scope {
    let lease = control.acquire_instance(BTreeMap::new()).await.unwrap();
    control.refresh_configuration().await.unwrap().unwrap();
    let snapshot = control.execution_snapshot().unwrap();
    let profile = snapshot.configuration().unwrap().dns.profile.clone();
    let endpoints: Vec<SocketAddr> = vec![
        "192.0.2.53:53".parse().unwrap(),
        "192.0.2.54:53".parse().unwrap(),
    ];
    let mut demands = BTreeMap::new();
    let mut tcp_demands = BTreeMap::new();
    for (index, endpoint) in endpoints.iter().enumerate() {
        for protocol in [BusinessProtocol::Udp, BusinessProtocol::Tcp] {
            let id = request_id();
            control
                .create_demand(&DemandRequest {
                    instance: InstanceRequest {
                        instance_id: lease.instance_id,
                        generation: lease.generation,
                    },
                    request_id: id,
                    target: BusinessTarget::Ip {
                        address: ExecutionAddress(endpoint.ip()),
                    },
                    protocol,
                    port: Some(53.try_into().unwrap()),
                    profile: profile.clone(),
                    duration_seconds: 60,
                })
                .await
                .unwrap();
            if protocol == BusinessProtocol::Udp {
                demands.insert(index, id);
            } else {
                tcp_demands.insert(index, id);
            }
        }
    }
    control.refresh_configuration().await.unwrap().unwrap();
    let snapshot = control.execution_snapshot().unwrap();
    let scope = Scope {
        snapshot,
        resolver_demands: BTreeMap::new(),
        // A private provenance fixture only: endpoint permission still comes
        // exclusively from the real Server/Control publication above.
        system_resolvers: Some(SystemResolverSnapshot {
            id: "dispatcher-provenance-fixture".into(),
            upstreams: Arc::new(endpoints),
            primary: DnsTransport::Udp,
            invalid: CancellationToken::new(),
            verified_until: Arc::new(StdMutex::new(Instant::now() + Duration::from_secs(30))),
        }),
        system_demands: demands,
        system_tcp_demands: tcp_demands,
    };
    scope.validate_resolvers().expect("fixture must grant all four exact system routes and have no additional configured resolver");
    scope
}

fn make_dispatcher(
    scope: Scope,
    exchange: Arc<Exchange>,
) -> (OwnerDispatcher, watch::Sender<Option<Scope>>) {
    let (sender, receiver) = watch::channel(Some(scope));
    let dispatcher = OwnerDispatcher::new(
        receiver,
        Arc::new(NoDiscovery),
        exchange,
        Limits {
            max_pending: 1,
            query_timeout: Duration::from_secs(2),
        },
        1232,
    )
    .unwrap();
    (dispatcher, sender)
}

#[tokio::test]
async fn real_dispatcher_system_failover_keeps_ticket_authority_and_original_budget() {
    let server = support::Server::start().await;
    let mut control = server.control();
    // Preserve assertion failures while still settling the Control writer,
    // joining HTTP tasks and shutting down the real application/database owner.
    let outcome = support::catch(async {
        server.enroll(&mut control).await;
        check_system_failover(&mut control).await;
    })
    .await;
    let control_shutdown = control.shutdown().await;
    server.close().await;
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
    control_shutdown.unwrap();
}

async fn check_system_failover(control: &mut Control) {
    let scope = fixture(control).await;
    let first: SocketAddr = "192.0.2.53:53".parse().unwrap();
    let second: SocketAddr = "192.0.2.54:53".parse().unwrap();
    // The first silent server gets only half of this original client budget.
    // The second's UDP truncation retries TCP on that same selected endpoint.
    let exchange = Exchange::new(&[Step::Hang, Step::Truncated, Step::Answer]);
    let (dispatcher, _sender) = make_dispatcher(scope.clone(), exchange.clone());
    let deadline = Instant::now() + Duration::from_millis(400);
    let (answer, ()) = tokio::join!(dispatcher.dispatch(query(), deadline), async {
        exchange.entered(1).await;
        let state = dispatcher.state.lock().await;
        assert_eq!(state.owner.as_ref().unwrap().pending_len(), 1);
    });
    let answer = answer.unwrap();
    assert_eq!(
        exchange.endpoints(),
        vec![
            (first, DnsTransport::Udp),
            (second, DnsTransport::Udp),
            (second, DnsTransport::Tcp)
        ]
    );
    {
        let calls = exchange.calls.lock().unwrap();
        assert!(calls[0].deadline() < deadline);
        assert!(calls.iter().all(|r| r.deadline() <= deadline));
        assert_eq!(
            calls[1].deadline(),
            calls[2].deadline(),
            "TCP must not reset its selected upstream's budget"
        );
        assert!(
            calls.iter().all(|r| r.ensure_current().is_err()),
            "finished/abandoned exchange child guards must be cancelled"
        );
    }
    assert!(
        answer.ensure_current().is_ok(),
        "exchange cancellation must not cancel the query ticket"
    );
    tokio::time::timeout_at(
        (deadline + Duration::from_secs(1)).into(),
        answer.invalidated(),
    )
    .await
    .unwrap();
    assert!(answer.ensure_current().is_err());
    assert_eq!(
        dispatcher
            .state
            .lock()
            .await
            .owner
            .as_ref()
            .unwrap()
            .pending_len(),
        0
    );
    dispatcher.stop().await;

    // TCP-only OS resolution skips UDP, while an immediate network failure
    // leaves the remaining original budget available to the next server.
    let mut tcp_scope = scope.clone();
    tcp_scope.system_resolvers.as_mut().unwrap().primary = DnsTransport::Tcp;
    tcp_scope.system_demands = scope.system_tcp_demands.clone();
    tcp_scope.system_tcp_demands.clear();
    let exchange = Exchange::new(&[Step::Fail, Step::Answer]);
    let (dispatcher, _sender) = make_dispatcher(tcp_scope, exchange.clone());
    assert!(
        dispatcher
            .dispatch(query(), Instant::now() + Duration::from_secs(1))
            .await
            .is_ok()
    );
    assert_eq!(
        exchange.endpoints(),
        vec![(first, DnsTransport::Tcp), (second, DnsTransport::Tcp)]
    );
    dispatcher.stop().await;

    // A failed TCP retry cannot jump to another server's TCP endpoint. The
    // next independently selected server starts with the original OS transport.
    let exchange = Exchange::new(&[Step::Truncated, Step::Fail, Step::Answer]);
    let (dispatcher, _sender) = make_dispatcher(scope.clone(), exchange.clone());
    assert!(
        dispatcher
            .dispatch(query(), Instant::now() + Duration::from_secs(1))
            .await
            .is_ok()
    );
    assert_eq!(
        exchange.endpoints(),
        vec![
            (first, DnsTransport::Udp),
            (first, DnsTransport::Tcp),
            (second, DnsTransport::Udp)
        ]
    );
    dispatcher.stop().await;

    // Every server is attempted at most once per query. A failed query's
    // cancellation also frees the single pending slot for the next query.
    let exchange = Exchange::new(&[Step::Fail, Step::Fail, Step::Answer]);
    let (dispatcher, _sender) = make_dispatcher(scope.clone(), exchange.clone());
    assert!(matches!(
        dispatcher
            .dispatch(query(), Instant::now() + Duration::from_secs(1))
            .await,
        Err(DispatchError::Gateway)
    ));
    assert_eq!(
        exchange.endpoints(),
        vec![(first, DnsTransport::Udp), (second, DnsTransport::Udp)]
    );
    assert!(
        dispatcher
            .dispatch(query(), Instant::now() + Duration::from_secs(1))
            .await
            .is_ok()
    );
    dispatcher.stop().await;

    // Missing, wrong-endpoint and wrong-transport mappings fail before any I/O;
    // neither a reachable first server nor a later permitted one excuses them.
    for case in 0..3 {
        let mut bad = scope.clone();
        match case {
            0 => {
                bad.system_demands.remove(&1);
            }
            1 => {
                bad.system_demands.insert(0, scope.system_demands[&1]);
            }
            2 => {
                bad.system_tcp_demands.insert(1, scope.system_demands[&1]);
            }
            _ => unreachable!(),
        }
        let exchange = Exchange::new(&[]);
        let (dispatcher, _sender) = make_dispatcher(bad, exchange.clone());
        assert!(matches!(
            dispatcher
                .dispatch(query(), Instant::now() + Duration::from_secs(1))
                .await,
            Err(DispatchError::Unavailable(Unavailable::MissingResolverPlan))
        ));
        assert!(exchange.endpoints().is_empty());
        dispatcher.stop().await;
    }

    // Replacing a live scope fences both unsent answers and blocked work,
    // even when the published snapshot itself is unchanged.
    let exchange = Exchange::new(&[Step::Answer, Step::Hang, Step::Answer]);
    let (dispatcher, sender) = make_dispatcher(scope.clone(), exchange.clone());
    let answer = dispatcher
        .dispatch(query(), Instant::now() + Duration::from_secs(1))
        .await
        .unwrap();
    let (result, ()) = tokio::join!(
        dispatcher.dispatch(query(), Instant::now() + Duration::from_secs(1)),
        async {
            exchange.entered(2).await;
            sender.send_replace(Some(scope.clone()));
            dispatcher.reconcile().await.unwrap();
        }
    );
    assert!(result.is_err());
    assert!(answer.ensure_current().is_err());
    assert!(exchange.calls.lock().unwrap()[1].ensure_current().is_err());
    assert!(
        dispatcher
            .dispatch(query(), Instant::now() + Duration::from_secs(1))
            .await
            .is_ok()
    );
    dispatcher.stop().await;

    // Both watch withdrawal and actual Control revocation must stop a blocked
    // first exchange rather than letting its timeout start the second server.
    for revoke_control in [false, true] {
        let exchange = Exchange::new(&[Step::Hang]);
        let (dispatcher, sender) = make_dispatcher(scope.clone(), exchange.clone());
        let (result, ()) = tokio::join!(
            dispatcher.dispatch(query(), Instant::now() + Duration::from_secs(1)),
            async {
                exchange.entered(1).await;
                if revoke_control {
                    control.stop().await.unwrap();
                } else {
                    sender.send_replace(None);
                }
            }
        );
        assert!(matches!(
            result,
            Err(DispatchError::Scope | DispatchError::Expired)
        ));
        assert_eq!(exchange.endpoints(), vec![(first, DnsTransport::Udp)]);
        assert!(exchange.calls.lock().unwrap()[0].ensure_current().is_err());
        dispatcher.stop().await;
    }
}
