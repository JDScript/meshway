//! A source releases a name-rule answer only once an association covering it
//! is applied, with its TTLs capped; otherwise SERVFAIL, never the addresses.
use super::*;
use hickory_proto::rr::{RData, Record, rdata};
use meshway_configuration::{
    execution::ExecutionAddress,
    netmap::{
        AssociationRecordType, DnsAssociation, DnsAssociationOverlay, MAX_ASSOCIATION_ADDRESSES,
    },
};

/// The exit's answer to every Mesh query: this many public A records from
/// 104.16.0.9 up, TTL 300.
struct MeshAnswer(u8);
impl ResolverExchange for MeshAnswer {
    fn validate_mesh(&self, _: &MeshResolverRoute) -> Result<(), GatewayError> {
        Ok(())
    }
    fn exchange_mesh<'a>(
        &'a self,
        _: &'a MeshResolverRoute,
        bytes: Bytes,
    ) -> DnsFuture<'a, Result<Bytes, GatewayError>> {
        Box::pin(async move {
            let query = decode_query(&bytes).unwrap();
            let mut response = query.response();
            for index in 0..self.0 {
                response.add_answer(Record::from_rdata(
                    query.question().name().clone(),
                    300,
                    RData::A(rdata::A(std::net::Ipv4Addr::new(104, 16, 0, 9 + index))),
                ));
            }
            Ok(Bytes::from(response.to_vec().unwrap()))
        })
    }
    fn exchange<'a>(
        &'a self,
        _: &'a LocalResolverRoute,
        _: Bytes,
    ) -> DnsFuture<'a, Result<Bytes, GatewayError>> {
        Box::pin(async { Err(GatewayError) })
    }
}

fn ttls(answer: &Answer) -> Vec<u32> {
    answer
        .message()
        .answers
        .iter()
        .map(|record| record.ttl)
        .collect()
}

#[tokio::test]
async fn a_name_rule_answer_waits_for_its_association_and_never_leaks_without_one() {
    let server = support::Server::start().await;
    let (mut control, script) = enrolled(&server).await;
    let mut network = Network::new(&control, 'a');
    network.name_rule = true;
    let netmap = network.compile(None);
    assert!(matches!(netmap.netmap.dns.egress, DnsPlan::ViaExit { .. }));
    let desired = sync(&mut control, &script, reply(&netmap, true)).await;
    let (scopes, received) = watch::channel(None);
    crate::publish_scope(&scopes, Some(Scope::netmap(desired.clone(), None)));
    let dispatcher = OwnerDispatcher::new(received, Arc::new(MeshAnswer(1)), limits(), 1232).unwrap();
    let deadline = || Instant::now() + Duration::from_secs(4);

    // No name rule decides it: released at once, unchanged.
    let plain = dispatcher
        .dispatch(named("example.org."), deadline())
        .await
        .unwrap();
    assert_eq!(ttls(&plain), [300]);

    // No association arrives: SERVFAIL (an error) after the bounded wait.
    let started = Instant::now();
    assert_eq!(
        dispatcher
            .dispatch(named("other.chat.example."), deadline())
            .await
            .unwrap_err(),
        DispatchError::Expired
    );
    assert!(started.elapsed() >= crate::ASSOCIATION_WAIT);

    // The association arrives while the answer is held: released, TTL capped.
    let generation = desired.egress().unwrap().generation;
    let expires_at_ms = crate::association::wall_ms() + 60_000;
    let overlay = Arc::new(DnsAssociationOverlay {
        netmap: desired.reference(),
        associations: vec![DnsAssociation {
            source: desired.netmap().node_id,
            generation,
            name: "api.chat.example".into(),
            record_type: AssociationRecordType::A,
            cnames: vec![],
            addresses: vec![ExecutionAddress("104.16.0.9".parse().unwrap())],
            expires_at_ms,
        }],
    });
    let associated = desired
        .with_associations(Some(&overlay), crate::association::wall_ms())
        .unwrap();
    let publisher = async {
        tokio::time::sleep(Duration::from_millis(300)).await;
        crate::publish_scope(&scopes, Some(Scope::netmap(associated.clone(), None)));
    };
    let (answer, ()) = tokio::join!(
        dispatcher.dispatch(named("API.chat.example."), deadline()),
        publisher
    );
    let answer = answer.unwrap();
    assert!(
        ttls(&answer).iter().all(|ttl| (50..=58).contains(ttl)),
        "{:?}",
        ttls(&answer)
    );
    // Covered now: no wait at all.
    let started = Instant::now();
    dispatcher
        .dispatch(named("api.chat.example."), deadline())
        .await
        .unwrap();
    assert!(started.elapsed() < Duration::from_millis(500));
    let _ = control.shutdown().await;
    server.close().await;
}

#[tokio::test]
async fn a_name_rule_answer_beyond_one_association_is_refused_not_cut() {
    let server = support::Server::start().await;
    let (mut control, script) = enrolled(&server).await;
    let mut network = Network::new(&control, 'a');
    network.name_rule = true;
    let netmap = network.compile(None);
    let desired = sync(&mut control, &script, reply(&netmap, true)).await;
    let count = MAX_ASSOCIATION_ADDRESSES as u8 + 1;
    // Even an association naming the first sixteen leaves the seventeenth
    // uncovered: it would leave by the IP fallback.
    let first: Vec<_> = (0..count - 1)
        .map(|index| ExecutionAddress(std::net::Ipv4Addr::new(104, 16, 0, 9 + index).into()))
        .collect();
    let overlay = Arc::new(DnsAssociationOverlay {
        netmap: desired.reference(),
        associations: vec![DnsAssociation {
            source: desired.netmap().node_id,
            generation: desired.egress().unwrap().generation,
            name: "big.chat.example".into(),
            record_type: AssociationRecordType::A,
            cnames: vec![],
            addresses: first,
            expires_at_ms: crate::association::wall_ms() + 60_000,
        }],
    });
    let associated = desired
        .with_associations(Some(&overlay), crate::association::wall_ms())
        .unwrap();
    let (scopes, received) = watch::channel(None);
    crate::publish_scope(&scopes, Some(Scope::netmap(associated, None)));
    let dispatcher =
        OwnerDispatcher::new(received, Arc::new(MeshAnswer(count)), limits(), 1232).unwrap();
    let deadline = || Instant::now() + Duration::from_secs(4);
    let started = Instant::now();
    assert_eq!(
        dispatcher
            .dispatch(named("big.chat.example."), deadline())
            .await
            .unwrap_err(),
        DispatchError::Expired
    );
    assert!(started.elapsed() < crate::ASSOCIATION_WAIT);
    // No name rule decides it: released whole.
    let plain = dispatcher
        .dispatch(named("big.example.org."), deadline())
        .await
        .unwrap();
    assert_eq!(plain.message().answers.len(), usize::from(count));
    let _ = control.shutdown().await;
    server.close().await;
}
