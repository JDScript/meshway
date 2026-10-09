//! Real HTTP enrollment and Control publications feed DNS's public owner API.
#![cfg(unix)]

mod support;

use meshway_model::NodeId;
use std::time::Duration;
use support::{Server, WAIT, dns};

#[tokio::test]
async fn real_control_dns_discovery_binds_wire_answers_and_preserves_typed_not_visible() {
    use hickory_proto::{
        op::{Message, ResponseCode},
        rr::{RData, RecordType},
    };
    use meshway_dns::{Begin, Owner, ResponseTransport, Unavailable};
    let server = Server::start().await;
    let mut node = server.enroll("dns-self").await;
    server.deny_access().await;
    // A changed configuration is saved successfully without minting a run
    // permit. The following host helper must explicitly renew that lease.
    node.control.sync_configuration().await.unwrap().unwrap();
    assert!(matches!(
        node.control.execution_snapshot(),
        Err(meshway_control::Error::Expired)
    ));
    let snapshot = node.sync_current().await;
    let configuration = snapshot.configuration().unwrap();
    let name = meshway_configuration::node_dns_name(
        &configuration.hostname,
        &configuration.dns.settings.domain,
    )
    .unwrap();
    assert!(!configuration.dns.mesh_hosts.contains_key(&name));
    let addresses = configuration.addresses.clone();
    let absent = format!("absent.{}", configuration.dns.settings.domain);
    let mut owner = Owner::new(
        snapshot.clone(),
        meshway_dns::Limits {
            max_pending: 4,
            query_timeout: Duration::from_secs(10),
        },
    )
    .unwrap();
    for kind in [RecordType::A, RecordType::AAAA] {
        let request = dns::discover(&mut owner, &name, kind);
        let lease = node.control.status().instance;
        let response = node.control.query_dns(&request.name).await.unwrap();
        assert_eq!(node.control.status().instance, lease);
        let answer = dns::answer(owner.complete_discovery(&request.ticket, response));
        let bytes = answer.encode(ResponseTransport::Udp, 1232).unwrap();
        let message = Message::from_vec(&bytes).unwrap();
        assert_eq!(message.metadata.id, 123);
        assert_eq!(message.metadata.response_code, ResponseCode::NoError);
        assert_eq!(message.queries.len(), 1);
        assert_eq!(message.queries[0], *dns::query(&name, kind).question());
        assert_eq!(message.answers.len(), 1);
        let record = &message.answers[0];
        assert_eq!(record.ttl, 0);
        match (kind, &record.data) {
            (RecordType::A, RData::A(value)) => assert_eq!(Some(value.0), addresses.ipv4),
            (RecordType::AAAA, RData::AAAA(value)) => assert_eq!(Some(value.0), addresses.ipv6),
            other => panic!("unexpected self address: {other:?}"),
        }
        assert!(message.authorities.is_empty());
        // Completion consumes the ticket, while cancellation still cuts an
        // already constructed but unsent answer from that ticket.
        assert!(!owner.cancel(&request.ticket));
        tokio::time::timeout(WAIT, answer.invalidated())
            .await
            .unwrap();
        assert!(answer.encode(ResponseTransport::Udp, 1232).is_err());
    }
    let missing = dns::discover(&mut owner, &absent, RecordType::A);
    let response = node.control.query_dns(&missing.name).await.unwrap();
    assert!(response.record.is_none());
    dns::refused(owner.complete_discovery(&missing.ticket, response));

    let wrong = dns::discover(&mut owner, &name, RecordType::A);
    let mut response = node.control.query_dns(&wrong.name).await.unwrap();
    // Corrupt only the typed completion fixture; no fake credential or lease.
    response.source_node_id = NodeId::new(support::request().into_uuid()).unwrap();
    assert!(matches!(
        owner.complete_discovery(&wrong.ticket, response),
        Begin::Unavailable(Unavailable::BindingMismatch)
    ));
    let request = dns::discover(&mut owner, &name, RecordType::A);
    let response = node.control.query_dns(&request.name).await.unwrap();
    let answer = dns::answer(owner.complete_discovery(&request.ticket, response));
    // No outstanding Control operation/task exists here. Losing the unique
    // credential/lease owner must still fence this already completed answer.
    drop(node.control);
    tokio::time::timeout(WAIT, answer.invalidated())
        .await
        .unwrap();
    assert!(answer.ensure_current().is_err());
    assert!(answer.encode(ResponseTransport::Udp, 1232).is_err());
    server.close().await;
}

#[tokio::test]
async fn dns_owner_drop_and_control_stop_fence_late_real_completions() {
    use hickory_proto::rr::RecordType;
    use meshway_dns::{Begin, Owner, ResponseTransport, Unavailable};
    let server = Server::start().await;
    let mut node = server.enroll("dns-lifecycle").await;
    server.deny_access().await;
    let original = node.sync_current().await;
    let configuration = original.configuration().unwrap();
    let name = meshway_configuration::node_dns_name(
        &configuration.hostname,
        &configuration.dns.settings.domain,
    )
    .unwrap();
    let limits = meshway_dns::Limits {
        max_pending: 4,
        query_timeout: Duration::from_secs(10),
    };
    let mut owner = Owner::new(original.clone(), limits).unwrap();
    let request = dns::discover(&mut owner, &name, RecordType::A);
    let response = node.control.query_dns(&request.name).await.unwrap();
    let unsent = dns::answer(owner.complete_discovery(&request.ticket, response.clone()));
    drop(owner);
    tokio::time::timeout(WAIT, unsent.invalidated())
        .await
        .unwrap();
    assert!(unsent.encode(ResponseTransport::Udp, 1232).is_err());

    let mut owner = Owner::new(original, limits).unwrap();
    let request = dns::discover(&mut owner, &name, RecordType::A);
    let unsent = dns::answer(owner.complete_discovery(&request.ticket, response));
    let pending = dns::discover(&mut owner, &name, RecordType::A);
    let late_response = node.control.query_dns(&pending.name).await.unwrap();
    tokio::time::timeout(WAIT, async {
        let (stopped, ()) = tokio::join!(node.control.stop(), unsent.invalidated());
        stopped.unwrap();
    })
    .await
    .unwrap();
    assert!(unsent.ensure_current().is_err());
    assert!(unsent.encode(ResponseTransport::Udp, 1232).is_err());
    assert!(matches!(
        owner.complete_discovery(&pending.ticket, late_response),
        Begin::Unavailable(Unavailable::Inactive)
    ));
    node.control.shutdown().await.unwrap();
    server.close().await;
}
