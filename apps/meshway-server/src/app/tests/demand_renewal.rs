//! Absolute-deadline renewal against real enrollment, sessions and publications.
use super::{
    node::{caps, fixture, session},
    *,
};
use crate::{
    accounts::{Secret, random_uuid},
    error::Error,
    store,
};
use meshway_configuration::{ConfigurationOperation, execution::*};
use meshway_model::{InstanceId, NetworkId, NodeId, RequestId, Version};
use meshway_protocol::{execution::*, node::NodeSessionResponse};

fn rid() -> RequestId {
    RequestId::new(random_uuid().unwrap()).unwrap()
}

async fn acquire(app: &App, token: &NodeSessionResponse) -> InstanceLease {
    app.execution()
        .acquire(
            token.token.expose(),
            AcquireInstanceRequest {
                instance_id: InstanceId::new(random_uuid().unwrap()).unwrap(),
                capabilities: caps(),
            },
        )
        .await
        .unwrap()
}

fn instance(lease: &InstanceLease) -> InstanceRequest {
    InstanceRequest {
        instance_id: lease.instance_id,
        generation: lease.generation,
    }
}

fn demand(lease: &InstanceLease, target: NodeId) -> DemandRequest {
    DemandRequest {
        instance: instance(lease),
        request_id: rid(),
        target: BusinessTarget::Mesh { node: target },
        protocol: BusinessProtocol::Tcp,
        port: Some(443.try_into().unwrap()),
        profile: "mesh".into(),
        duration_seconds: 30,
    }
}

fn renewal(create: &DemandRequest, expires_at_ms: u64) -> RenewDemandRequest {
    RenewDemandRequest {
        instance: create.instance.clone(),
        request_id: create.request_id,
        expires_at_ms,
    }
}

async fn configuration(
    app: &App,
    token: &NodeSessionResponse,
    lease: &InstanceLease,
) -> NodeConfigurationResponse {
    app.execution()
        .configuration(
            token.token.expose(),
            NodeConfigurationRequest {
                instance: instance(lease),
                base: None,
            },
        )
        .await
        .unwrap()
}

#[derive(Debug, PartialEq)]
struct DemandRecord {
    payload: Vec<u8>,
    digest: Vec<u8>,
    expiry: i64,
    cancelled: Option<i64>,
}

#[derive(Debug, PartialEq)]
struct ExecutionRecord {
    node: Vec<u8>,
    target: Option<Vec<u8>>,
    instance_expiry: Option<i64>,
    execution_expiry: Option<i64>,
}

#[derive(Debug, PartialEq)]
struct Snapshot {
    publications: i64,
    demands: Vec<DemandRecord>,
    executions: Vec<ExecutionRecord>,
}

async fn snapshot(app: &App, network: NetworkId) -> Snapshot {
    app.executor.handle().execute(move |db| {
        let network = network.as_uuid().as_bytes().as_slice();
        let publications = db.query_row("SELECT count(*) FROM execution_publications WHERE network_id=?1", [network], |row| row.get(0)).unwrap();
        let demands = db.prepare("SELECT payload,request_digest,expires_at,cancelled_at FROM execution_demands WHERE network_id=?1 ORDER BY id").unwrap()
            .query_map([network], |row| Ok(DemandRecord { payload: row.get(0)?, digest: row.get(1)?, expiry: row.get(2)?, cancelled: row.get(3)? })).unwrap()
            .collect::<Result<Vec<_>, _>>().unwrap();
        let executions = db.prepare("SELECT node_id,target_publication_id,instance_expires_at,execution_expires_at FROM node_execution WHERE network_id=?1 ORDER BY node_id").unwrap()
            .query_map([network], |row| Ok(ExecutionRecord { node: row.get(0)?, target: row.get(1)?, instance_expiry: row.get(2)?, execution_expiry: row.get(3)? })).unwrap()
            .collect::<Result<Vec<_>, _>>().unwrap();
        Snapshot { publications, demands, executions }
    }).await.unwrap()
}

#[tokio::test]
async fn absolute_renewal_preserves_complete_targets_and_replays_without_writes() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, a, b, ak, bk, signer_a, signer_b) = fixture().await;
    let service = app.execution();
    let at = session(&service, network, a, &ak, &signer_a).await;
    let bt = session(&service, network, b, &bk, &signer_b).await;
    let ai = acquire(&app, &at).await;
    let bi = acquire(&app, &bt).await;
    let one = demand(&ai, b);
    let mut two = demand(&bi, a);
    two.duration_seconds = 60;
    service
        .demand(at.token.expose(), one.clone())
        .await
        .unwrap();
    service
        .demand(bt.token.expose(), two.clone())
        .await
        .unwrap();
    let ai = service
        .renew(at.token.expose(), instance(&ai))
        .await
        .unwrap();
    let before = configuration(&app, &at, &ai).await;
    let before_slice = &before.full.unwrap().configuration;
    let old_permit = before_slice.permits[&one.request_id].clone();
    let other_permit = before_slice.permits[&two.request_id].clone();
    let renew = renewal(&one, ai.expires_at_ms);
    let renewed = service
        .renew_demand(at.token.expose(), renew.clone())
        .await
        .unwrap();
    let DemandDecision::Permitted { permit } = &renewed.decision else {
        panic!("permitted renewal")
    };
    assert_eq!(permit.expires_at_ms, ai.expires_at_ms);
    assert!(permit.expires_at_ms > old_permit.expires_at_ms);
    assert_eq!(permit.paths, old_permit.paths);
    assert_ne!(
        renewed.publication,
        before.lease.target.unwrap().execution_publication
    );
    for (token, lease) in [(&at, &ai), (&bt, &bi)] {
        let current = configuration(&app, token, lease).await;
        let slice = current.full.unwrap().configuration;
        assert_eq!(slice.permits.len(), 2);
        assert_eq!(slice.permits[&one.request_id], *permit);
        assert_eq!(slice.permits[&two.request_id], other_permit);
        assert_eq!(current.lease.expires_at_ms, lease.expires_at_ms);
    }
    let before_retry = snapshot(&app, network).await;
    let event_sequence = app.state_runtime.lock().unwrap().hub.events.sequence();
    for expiry in [renew.expires_at_ms, old_permit.expires_at_ms, 0] {
        assert_eq!(
            service
                .renew_demand(at.token.expose(), renewal(&one, expiry))
                .await
                .unwrap(),
            renewed
        );
    }
    // The original create hash remains usable, including after a newer renewal.
    assert_eq!(
        service
            .demand(at.token.expose(), one.clone())
            .await
            .unwrap(),
        renewed
    );
    assert_eq!(snapshot(&app, network).await, before_retry);
    assert_eq!(
        app.state_runtime.lock().unwrap().hub.events.sequence(),
        event_sequence
    );
    let mut conflicting = one;
    conflicting.duration_seconds += 1;
    assert_eq!(
        service.demand(at.token.expose(), conflicting).await,
        Err(Error::BindingConflict)
    );
    app.check_storage().await.unwrap();
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn renewal_is_instance_bounded_and_cannot_resurrect_missing_cancelled_or_expired_intents() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, a, b, ak, bk, signer_a, signer_b) = fixture().await;
    let service = app.execution();
    let at = session(&service, network, a, &ak, &signer_a).await;
    let bt = session(&service, network, b, &bk, &signer_b).await;
    let ai = acquire(&app, &at).await;
    let bi = acquire(&app, &bt).await;
    let create = demand(&ai, b);
    service
        .demand(at.token.expose(), create.clone())
        .await
        .unwrap();
    let before = snapshot(&app, network).await;
    for expires in [ai.expires_at_ms + 1, u64::MAX] {
        assert_eq!(
            service
                .renew_demand(at.token.expose(), renewal(&create, expires))
                .await,
            Err(Error::InvalidRequest)
        );
    }
    let mut wrong = renewal(&create, ai.expires_at_ms);
    wrong.instance.generation = Version::new(ai.generation.get() + 1).unwrap();
    assert_eq!(
        service.renew_demand(at.token.expose(), wrong).await,
        Err(Error::BindingConflict)
    );
    let mut foreign = renewal(&create, bi.expires_at_ms);
    foreign.instance = instance(&bi);
    assert_eq!(
        service.renew_demand(bt.token.expose(), foreign).await,
        Err(Error::BindingConflict)
    );
    let missing = demand(&ai, b);
    assert_eq!(
        service
            .renew_demand(at.token.expose(), renewal(&missing, ai.expires_at_ms))
            .await,
        Err(Error::BindingConflict)
    );
    assert_eq!(snapshot(&app, network).await, before);
    service
        .cancel_demand(
            at.token.expose(),
            CancelDemandRequest {
                instance: instance(&ai),
                request_id: create.request_id,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        service
            .renew_demand(at.token.expose(), renewal(&create, ai.expires_at_ms))
            .await,
        Err(Error::BindingConflict)
    );
    let expired = demand(&ai, b);
    service
        .demand(at.token.expose(), expired.clone())
        .await
        .unwrap();
    let request_id = expired.request_id;
    app.executor
        .handle()
        .execute(move |db| {
            db.execute(
                "UPDATE execution_demands SET expires_at=0 WHERE id=?1",
                [request_id.as_uuid().as_bytes().as_slice()],
            )
            .unwrap();
        })
        .await
        .unwrap();
    // The storage owner serializes cleanup and renewal in either accepted order.
    let (renewed, cleaned) = tokio::join!(
        service.renew_demand(at.token.expose(), renewal(&expired, ai.expires_at_ms)),
        service.renew(bt.token.expose(), instance(&bi)),
    );
    assert_eq!(renewed, Err(Error::BindingConflict));
    cleaned.unwrap();
    for (token, lease) in [(&at, &ai), (&bt, &bi)] {
        assert!(
            configuration(&app, token, lease)
                .await
                .full
                .unwrap()
                .configuration
                .permits
                .is_empty()
        );
    }
    assert_eq!(
        service
            .renew_demand(at.token.expose(), renewal(&expired, 0))
            .await,
        Err(Error::BindingConflict)
    );
    let records = snapshot(&app, network).await;
    assert!(
        records
            .demands
            .iter()
            .all(|record| record.cancelled.is_some())
    );
    let fresh = demand(&ai, b);
    service
        .demand(at.token.expose(), fresh.clone())
        .await
        .unwrap();
    let next = acquire(&app, &at).await;
    assert_eq!(
        service
            .renew_demand(at.token.expose(), renewal(&fresh, next.expires_at_ms))
            .await,
        Err(Error::BindingConflict)
    );
    let mut replaced = renewal(&fresh, next.expires_at_ms);
    replaced.instance = instance(&next);
    assert_eq!(
        service.renew_demand(at.token.expose(), replaced).await,
        Err(Error::BindingConflict)
    );
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn renewal_recompiles_current_access_and_never_falls_back_or_revives_dns() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, a, b, ak, bk, signer_a, signer_b) = fixture().await;
    let service = app.execution();
    let at = session(&service, network, a, &ak, &signer_a).await;
    let bt = session(&service, network, b, &bk, &signer_b).await;
    let ai = acquire(&app, &at).await;
    acquire(&app, &bt).await;
    let create = demand(&ai, b);
    assert!(matches!(
        service
            .demand(at.token.expose(), create.clone())
            .await
            .unwrap()
            .decision,
        DemandDecision::Permitted { .. }
    ));
    let owner = app
        .accounts()
        .login(
            "owner@example.com".into(),
            Secret::new("a sufficiently long password".into()),
            rid(),
        )
        .await
        .unwrap();
    let revision = app
        .networks()
        .get(owner.token.expose(), network)
        .await
        .unwrap()
        .current_revision;
    app.configurations()
        .apply(
            owner.token.expose(),
            network,
            meshway_protocol::configuration::ConfigurationRequest::Operations {
                expected_revision: revision,
                request_id: rid(),
                ops: vec![ConfigurationOperation::ReplaceAccess { access: vec![] }],
            },
            rid(),
        )
        .await
        .unwrap();
    let denied = DemandDecision::Unavailable {
        reason: UnavailableReason::Unauthorized,
    };
    assert_eq!(
        service
            .renew_demand(at.token.expose(), renewal(&create, ai.expires_at_ms))
            .await
            .unwrap()
            .decision,
        denied
    );
    let before = snapshot(&app, network).await;
    assert_eq!(
        service
            .renew_demand(at.token.expose(), renewal(&create, ai.expires_at_ms))
            .await
            .unwrap()
            .decision,
        denied
    );
    assert_eq!(snapshot(&app, network).await, before);
    assert!(
        configuration(&app, &at, &ai)
            .await
            .full
            .unwrap()
            .configuration
            .permits
            .is_empty()
    );
    let mut domain = demand(&ai, b);
    domain.target = BusinessTarget::Domain {
        name: "api.example.com".into(),
        address: ExecutionAddress("1.1.1.1".parse().unwrap()),
    };
    let missing_dns = DemandDecision::Unavailable {
        reason: UnavailableReason::MissingDns,
    };
    assert_eq!(
        service
            .demand(at.token.expose(), domain.clone())
            .await
            .unwrap()
            .decision,
        missing_dns
    );
    assert_eq!(
        service
            .renew_demand(at.token.expose(), renewal(&domain, ai.expires_at_ms))
            .await
            .unwrap()
            .decision,
        missing_dns
    );
    assert!(
        configuration(&app, &at, &ai)
            .await
            .full
            .unwrap()
            .configuration
            .permits
            .is_empty()
    );
    app.check_storage().await.unwrap();
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn renewal_and_replay_remove_expired_siblings_from_every_participant() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, a, b, ak, bk, signer_a, signer_b) = fixture().await;
    let service = app.execution();
    let at = session(&service, network, a, &ak, &signer_a).await;
    let bt = session(&service, network, b, &bk, &signer_b).await;
    let ai = acquire(&app, &at).await;
    let bi = acquire(&app, &bt).await;
    let live = demand(&ai, b);
    service
        .demand(at.token.expose(), live.clone())
        .await
        .unwrap();
    for _ in 0..2 {
        let sibling = demand(&bi, a);
        service
            .demand(bt.token.expose(), sibling.clone())
            .await
            .unwrap();
        let request_id = sibling.request_id;
        app.executor
            .handle()
            .execute(move |db| {
                db.execute(
                    "UPDATE execution_demands SET expires_at=0 WHERE id=?1",
                    [request_id.as_uuid().as_bytes().as_slice()],
                )
                .unwrap();
            })
            .await
            .unwrap();
        // First iteration extends the deadline; the second repeats it exactly.
        let response = service
            .renew_demand(at.token.expose(), renewal(&live, ai.expires_at_ms))
            .await
            .unwrap();
        assert!(matches!(
            response.decision,
            DemandDecision::Permitted { .. }
        ));
        for (token, lease) in [(&at, &ai), (&bt, &bi)] {
            let target = configuration(&app, token, lease).await.full.unwrap();
            assert_eq!(target.configuration.permits.len(), 1);
            assert!(target.configuration.permits.contains_key(&live.request_id));
            assert!(
                !target
                    .configuration
                    .decisions
                    .contains_key(&sibling.request_id)
            );
        }
        assert_eq!(
            service
                .renew_demand(bt.token.expose(), renewal(&sibling, bi.expires_at_ms))
                .await,
            Err(Error::BindingConflict)
        );
    }
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn renewal_checks_current_credentials_and_network_in_its_final_transaction() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    // Each independently enrolled fixture ends permanently. Authentication done
    // before revocation cannot replace the renewal's final-transaction checks.
    for invalidate in [
        "UPDATE node_sessions SET created_at=0,expires_at=1",
        "UPDATE node_keys SET revoked_at=created_at WHERE purpose='control'",
        "UPDATE nodes SET credentials_revoked_at=registered_at",
        "UPDATE networks SET deleted_at=created_at",
    ] {
        let (_dir, app, network, a, b, ak, _, signer_a, _) = fixture().await;
        let service = app.execution();
        let at = session(&service, network, a, &ak, &signer_a).await;
        let ai = acquire(&app, &at).await;
        let create = demand(&ai, b);
        service
            .demand(at.token.expose(), create.clone())
            .await
            .unwrap();
        service.authenticate(at.token.expose()).await.unwrap();
        let before = snapshot(&app, network).await;
        app.executor
            .handle()
            .execute(move |db| {
                db.execute_batch(invalidate).unwrap();
            })
            .await
            .unwrap();
        assert_eq!(
            service
                .renew_demand(at.token.expose(), renewal(&create, ai.expires_at_ms))
                .await,
            Err(Error::InvalidCredentials)
        );
        assert_eq!(snapshot(&app, network).await, before);
        app.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn renewal_enforces_maximum_window_even_with_a_later_instance_deadline() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, a, b, ak, _, signer_a, _) = fixture().await;
    let service = app.execution();
    let at = session(&service, network, a, &ak, &signer_a).await;
    let ai = acquire(&app, &at).await;
    let create = demand(&ai, b);
    service
        .demand(at.token.expose(), create.clone())
        .await
        .unwrap();
    let later = store::execution::now().unwrap() + 600_000;
    app.executor.handle().execute(move |db| {
        db.execute("UPDATE node_execution SET instance_expires_at=?1 WHERE network_id=?2 AND node_id=?3", rusqlite::params![later, network.as_uuid().as_bytes().as_slice(), a.as_uuid().as_bytes().as_slice()]).unwrap();
    }).await.unwrap();
    let before = snapshot(&app, network).await;
    assert_eq!(
        service
            .renew_demand(at.token.expose(), renewal(&create, later as u64))
            .await,
        Err(Error::InvalidRequest)
    );
    assert_eq!(snapshot(&app, network).await, before);
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn another_network_cannot_renew_a_demand_with_its_own_authenticated_instance() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, a, b, ak, _, signer_a, _) = fixture().await;
    let service = app.execution();
    let at = session(&service, network, a, &ak, &signer_a).await;
    let ai = acquire(&app, &at).await;
    let create = demand(&ai, b);
    service
        .demand(at.token.expose(), create.clone())
        .await
        .unwrap();
    let owner = app
        .accounts()
        .login(
            "owner@example.com".into(),
            Secret::new("a sufficiently long password".into()),
            rid(),
        )
        .await
        .unwrap();
    let other = app
        .networks()
        .create(owner.token.expose(), "other".into(), rid())
        .await
        .unwrap();
    app.configurations()
        .apply(
            owner.token.expose(),
            other.id,
            meshway_protocol::configuration::ConfigurationRequest::Operations {
                expected_revision: other.current_revision,
                request_id: rid(),
                ops: vec![ConfigurationOperation::ReplaceTagsOwners {
                    tags_owners: std::collections::BTreeMap::from([("tag:test".into(), vec![])]),
                }],
            },
            rid(),
        )
        .await
        .unwrap();
    let signer = ed25519_dalek::SigningKey::from_bytes(&[99; 32]);
    let enrolled = super::node::enroll_tag_node(
        &app,
        owner.token.expose(),
        other.id,
        &signer,
        "other",
        [98; 32],
    )
    .await;
    let other_token = session(
        &service,
        other.id,
        enrolled.node_id,
        &enrolled.keys,
        &signer,
    )
    .await;
    let other_lease = acquire(&app, &other_token).await;
    let before = snapshot(&app, network).await;
    let mut request = renewal(&create, other_lease.expires_at_ms);
    request.instance = instance(&other_lease);
    assert_eq!(
        service
            .renew_demand(other_token.token.expose(), request)
            .await,
        Err(Error::BindingConflict)
    );
    assert_eq!(snapshot(&app, network).await, before);
    app.check_storage().await.unwrap();
    app.shutdown().await.unwrap();
}

#[test]
fn renewal_route_uses_node_session_and_an_absolute_timestamp_contract() {
    let spec = serde_json::to_value(crate::http::openapi()).unwrap();
    let route = &spec["paths"]["/agent-api/v1/demands/renew"]["post"];
    assert_eq!(route["operationId"], "renewNodeDemand");
    assert_eq!(
        route["security"],
        serde_json::json!([{ "nodeSession": [] }])
    );
    assert_eq!(
        route["requestBody"]["content"]["application/json"]["schema"]["$ref"],
        "#/components/schemas/RenewDemandRequest"
    );
    assert_eq!(
        spec["components"]["schemas"]["RenewDemandRequest"]["properties"]["expiresAt"]["$ref"],
        "#/components/schemas/Timestamp"
    );
    assert_eq!(
        spec["components"]["schemas"]["Timestamp"]["format"],
        "meshway-timestamp-ms"
    );
}

#[tokio::test]
async fn local_egress_renewal_keeps_exact_binding_and_uses_current_routing() {
    use meshway_configuration::{EgressRule, RouteMatch, RoutingConfiguration, RuleAction};
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, a, b, ak, bk, signer_a, signer_b) = fixture().await;
    let owner = app
        .accounts()
        .login(
            "owner@example.com".into(),
            Secret::new("a sufficiently long password".into()),
            rid(),
        )
        .await
        .unwrap();
    let mut routing = RoutingConfiguration::default();
    routing.egress.rules.push(EgressRule {
        remark: String::new(),
        matcher: RouteMatch {
            ip_cidr: vec!["1.1.1.1/32".into()],
            ..Default::default()
        },
        action: RuleAction::Direct,
    });
    let revision = app
        .networks()
        .get(owner.token.expose(), network)
        .await
        .unwrap()
        .current_revision;
    let mut revision = app
        .configurations()
        .apply(
            owner.token.expose(),
            network,
            meshway_protocol::configuration::ConfigurationRequest::Operations {
                expected_revision: revision,
                request_id: rid(),
                ops: vec![ConfigurationOperation::ReplaceRouting { routing }],
            },
            rid(),
        )
        .await
        .unwrap()
        .revision;
    let service = app.execution();
    let at = session(&service, network, a, &ak, &signer_a).await;
    let bt = session(&service, network, b, &bk, &signer_b).await;
    let ai = acquire(&app, &at).await;
    let bi = acquire(&app, &bt).await;
    let mut create = demand(&ai, b);
    create.target = BusinessTarget::Ip {
        address: ExecutionAddress("1.1.1.1".parse().unwrap()),
    };
    create.protocol = BusinessProtocol::Udp;
    create.port = Some(53.try_into().unwrap());
    let first = service
        .demand(at.token.expose(), create.clone())
        .await
        .unwrap();
    assert_eq!(first.decision, DemandDecision::Direct);
    let original = configuration(&app, &at, &ai).await.full.unwrap();
    let original_binding = original.configuration.local_egress[&create.request_id].clone();
    let deadline = ai.expires_at_ms - 20_000;
    let renewed = service
        .renew_demand(at.token.expose(), renewal(&create, deadline))
        .await
        .unwrap();
    assert_eq!(renewed.decision, DemandDecision::Direct);
    assert_ne!(renewed.publication, first.publication);
    let current = configuration(&app, &at, &ai).await.full.unwrap();
    let binding = &current.configuration.local_egress[&create.request_id];
    assert_eq!(
        binding,
        &LocalEgressPermit {
            expires_at_ms: deadline,
            ..original_binding
        }
    );
    assert_eq!(binding.demand_id, create.request_id);
    assert_eq!(binding.source, a);
    assert_eq!(binding.target, create.target);
    assert_eq!(binding.protocol, create.protocol);
    assert_eq!(binding.port, create.port);
    assert_eq!(binding.profile, create.profile);
    assert_eq!(binding.action, LocalEgressAction::Direct);
    assert!(current.configuration.permits.is_empty());
    assert!(
        configuration(&app, &bt, &bi)
            .await
            .full
            .unwrap()
            .configuration
            .local_egress
            .is_empty()
    );
    let before = snapshot(&app, network).await;
    assert_eq!(
        service
            .renew_demand(at.token.expose(), renewal(&create, deadline))
            .await
            .unwrap(),
        renewed
    );
    assert_eq!(snapshot(&app, network).await, before);
    for foreign in [
        DemandRequest {
            target: BusinessTarget::Ip {
                address: ExecutionAddress("8.8.8.8".parse().unwrap()),
            },
            ..create.clone()
        },
        DemandRequest {
            port: Some(5353.try_into().unwrap()),
            ..create.clone()
        },
    ] {
        assert_eq!(
            service.demand(at.token.expose(), foreign).await,
            Err(Error::BindingConflict)
        );
    }
    revision = app
        .configurations()
        .apply(
            owner.token.expose(),
            network,
            meshway_protocol::configuration::ConfigurationRequest::Operations {
                expected_revision: revision,
                request_id: rid(),
                ops: vec![ConfigurationOperation::ReplaceRouting {
                    routing: RoutingConfiguration::default(),
                }],
            },
            rid(),
        )
        .await
        .unwrap()
        .revision;
    assert_eq!(
        service
            .renew_demand(at.token.expose(), renewal(&create, ai.expires_at_ms))
            .await
            .unwrap()
            .decision,
        DemandDecision::Unmanaged
    );
    let unmanaged = configuration(&app, &at, &ai).await.full.unwrap();
    assert_eq!(
        unmanaged.configuration.local_egress[&create.request_id].action,
        LocalEgressAction::Unmanaged
    );
    assert_eq!(
        unmanaged.configuration.local_egress[&create.request_id].expires_at_ms,
        ai.expires_at_ms
    );
    app.configurations()
        .apply(
            owner.token.expose(),
            network,
            meshway_protocol::configuration::ConfigurationRequest::Operations {
                expected_revision: revision,
                request_id: rid(),
                ops: vec![ConfigurationOperation::SetNodeSettings {
                    node_id: a,
                    fields: meshway_configuration::NodeSettingsPatch {
                        enabled: meshway_configuration::Input::Value(false),
                        ..Default::default()
                    },
                }],
            },
            rid(),
        )
        .await
        .unwrap();
    let disabled = configuration(&app, &at, &ai).await.full.unwrap();
    assert!(disabled.configuration.local_egress.is_empty());
    let delta = configuration_delta(&unmanaged, &disabled).unwrap();
    assert_eq!(delta.local_egress.remove, vec![create.request_id]);
    assert_eq!(
        apply_configuration_delta(&unmanaged, delta).unwrap(),
        disabled
    );
    app.check_storage().await.unwrap();
    app.shutdown().await.unwrap();
}
