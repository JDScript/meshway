//! A temporary real Server/Control verifies every netmap. Only the
//! endpoint exchange is scripted; no host DNS or kernel installation is used.
mod netmap;
mod support;
use super::*;
use crate::{SystemResolverSnapshot, decode_query};
use hickory_proto::{
    op::{Message, MessageType, OpCode, Query as Question},
    rr::{Name, RecordType},
};
use meshway_control::Control;
use std::{
    collections::{BTreeMap, VecDeque},
    net::SocketAddr,
    sync::Mutex as StdMutex,
    time::Duration,
};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

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

#[tokio::test]
async fn real_dispatcher_system_failover_keeps_ticket_authority_and_original_budget() {
    let server = support::Server::start().await;
    let mut control = server.real_control();
    server.enroll(&mut control).await;
    control
        .sync(&mut meshway_netmap_fixture::Facts::default())
        .await
        .unwrap();
    let snapshot = meshway_engine::Desired::new(&control.netmap().unwrap().content).unwrap();
    let endpoints: Vec<SocketAddr> = vec![
        "192.0.2.53:53".parse().unwrap(),
        "192.0.2.54:53".parse().unwrap(),
    ];
    let system = SystemResolverSnapshot {
        id: "system-provenance".into(),
        upstreams: Arc::new(endpoints.clone()),
        primary: DnsTransport::Udp,
        invalid: CancellationToken::new(),
        verified_until: Arc::new(StdMutex::new(Instant::now() + Duration::from_secs(30))),
    };
    let scope = Scope::netmap(snapshot, Some(system));
    let query = || {
        let mut message = Message::new(123, MessageType::Query, OpCode::Query);
        message.add_query(Question::query(
            Name::from_ascii("ordinary.example.").unwrap(),
            RecordType::A,
        ));
        decode_query(&message.to_vec().unwrap()).unwrap()
    };
    for (steps, expected) in [
        (
            [Step::Truncated, Step::Answer],
            vec![
                (endpoints[0], DnsTransport::Udp),
                (endpoints[0], DnsTransport::Tcp),
            ],
        ),
        (
            [Step::Fail, Step::Answer],
            vec![
                (endpoints[0], DnsTransport::Udp),
                (endpoints[1], DnsTransport::Udp),
            ],
        ),
    ] {
        let exchange = Exchange::new(&steps);
        let (_sender, receiver) = watch::channel(Some(scope.clone()));
        let dispatcher = OwnerDispatcher::new(
            receiver,
            exchange.clone(),
            Limits {
                max_pending: 1,
                query_timeout: Duration::from_secs(2),
            },
            1232,
        )
        .unwrap();
        let answer = dispatcher
            .dispatch(query(), Instant::now() + Duration::from_secs(2))
            .await
            .unwrap();
        answer.ensure_current().unwrap();
        assert_eq!(exchange.endpoints(), expected);
    }
    let exchange = Exchange::new(&[Step::Hang]);
    let (sender, receiver) = watch::channel(Some(scope));
    let dispatcher = OwnerDispatcher::new(
        receiver,
        exchange.clone(),
        Limits {
            max_pending: 1,
            query_timeout: Duration::from_secs(2),
        },
        1232,
    )
    .unwrap();
    let work = dispatcher.dispatch(query(), Instant::now() + Duration::from_secs(2));
    tokio::pin!(work);
    tokio::select! {
        result = &mut work => panic!("blocked resolver returned {result:?}"),
        _ = exchange.entered(1) => {}
    }
    crate::publish_scope(&sender, None);
    assert!(
        tokio::time::timeout(Duration::from_secs(1), work)
            .await
            .unwrap()
            .is_err()
    );
    assert!(exchange.calls.lock().unwrap()[0].ensure_current().is_err());
    control.shutdown().await.unwrap();
    server.close().await;
}
