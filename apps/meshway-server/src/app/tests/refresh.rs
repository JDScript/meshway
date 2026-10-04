//! One owner transaction binds the renewed lease to its exact serialized target.
use super::{
    node::{caps, enroll_user_node, fixture, session},
    *,
};
use crate::{
    accounts::{Secret, random_uuid},
    error::Error,
    store,
};
use meshway_configuration::{ConfigurationOperation, Input, NodeSettingsPatch, execution::*};
use meshway_model::{ConfigurationRef, InstanceId, NodeId, RequestId, Version};
use meshway_protocol::{execution::*, node::NodeSessionResponse};

fn rid() -> RequestId {
    RequestId::new(random_uuid().unwrap()).unwrap()
}
fn instance(lease: &InstanceLease) -> InstanceRequest {
    InstanceRequest {
        instance_id: lease.instance_id,
        generation: lease.generation,
    }
}
async fn acquire(app: &App, session: &NodeSessionResponse) -> InstanceLease {
    app.execution()
        .acquire(
            session.token.expose(),
            AcquireInstanceRequest {
                instance_id: InstanceId::new(random_uuid().unwrap()).unwrap(),
                capabilities: caps(),
            },
        )
        .await
        .unwrap()
}
fn query(lease: &InstanceLease, base: Option<ConfigurationRef>) -> NodeConfigurationRequest {
    NodeConfigurationRequest {
        instance: instance(lease),
        base,
    }
}
fn verify(
    response: &NodeConfigurationResponse,
    base: Option<&CompiledNode>,
    node: NodeId,
) -> CompiledNode {
    let target = match (&response.full, &response.delta) {
        (Some(full), None) => full.clone(),
        (None, Some(delta)) => apply_configuration_delta(base.unwrap(), delta.clone()).unwrap(),
        (None, None) => base.unwrap().clone(),
        _ => panic!("exclusive full/delta"),
    };
    assert_eq!(target.configuration.node_id, node);
    assert_eq!(
        meshway_configuration::digest(&target.configuration).unwrap(),
        target.digest
    );
    assert_eq!(
        response.lease.target,
        Some(ConfigurationRef {
            revision: target.configuration.revision,
            execution_publication: target.configuration.publication,
            digest: target.digest,
        })
    );
    if let Some(expires) = response.lease.execution_expires_at_ms {
        assert!(expires <= response.lease.expires_at_ms);
    }
    target
}

#[tokio::test]
async fn refresh_atomically_grants_complete_targets_while_query_remains_read_only() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, a, b, ak, bk, signer_a, signer_b) = fixture().await;
    let s = app.execution();
    let at = session(&s, network, a, &ak, &signer_a).await;
    let bt = session(&s, network, b, &bk, &signer_b).await;
    let ai = acquire(&app, &at).await;
    let bi = acquire(&app, &bt).await;
    let first = s
        .configuration(bt.token.expose(), query(&bi, None))
        .await
        .unwrap();
    let base = verify(&first, None, b);
    let mut demands = Vec::new();
    for port in [53, 443, 5353, 8443] {
        let request = DemandRequest {
            instance: instance(&ai),
            request_id: rid(),
            target: BusinessTarget::Mesh { node: b },
            protocol: BusinessProtocol::Udp,
            port: Some(port.try_into().unwrap()),
            profile: "mesh".into(),
            duration_seconds: 30,
        };
        s.demand(at.token.expose(), request.clone()).await.unwrap();
        demands.push(request);
    }
    let read = s
        .configuration(bt.token.expose(), query(&bi, None))
        .await
        .unwrap();
    assert_eq!(read.lease.expires_at_ms, bi.expires_at_ms);
    assert_eq!(read.lease.execution_expires_at_ms, None);
    assert_eq!(verify(&read, None, b).configuration.permits.len(), 4);
    let refreshed = s
        .refresh_configuration(bt.token.expose(), query(&bi, first.lease.target))
        .await
        .unwrap();
    assert!(refreshed.delta.is_some());
    let mut saved = verify(&refreshed, Some(&base), b);
    assert_eq!(saved.configuration.permits.len(), 4);
    assert!(
        refreshed.lease.execution_expires_at_ms.unwrap() > store::execution::now().unwrap() as u64
    );
    let same = s
        .refresh_configuration(bt.token.expose(), query(&bi, refreshed.lease.target))
        .await
        .unwrap();
    assert!(same.full.is_none() && same.delta.is_none());
    verify(&same, Some(&saved), b);

    // Bursts of source renewals interleave with receiver owner jobs. Every
    // returned body and lease must describe one immutable complete snapshot.
    for round in 0..3 {
        let renew = async {
            for request in &demands {
                s.renew_demand(
                    at.token.expose(),
                    RenewDemandRequest {
                        instance: instance(&ai),
                        request_id: request.request_id,
                        expires_at_ms: ai.expires_at_ms - 15_000 + round * 1000,
                    },
                )
                .await
                .unwrap();
            }
        };
        let refresh = async {
            for _ in 0..4 {
                let started = store::execution::now().unwrap() as u64;
                let response = s
                    .refresh_configuration(
                        bt.token.expose(),
                        query(
                            &bi,
                            Some(ConfigurationRef {
                                revision: saved.configuration.revision,
                                execution_publication: saved.configuration.publication,
                                digest: saved.digest,
                            }),
                        ),
                    )
                    .await
                    .unwrap();
                saved = verify(&response, Some(&saved), b);
                assert_eq!(saved.configuration.permits.len(), 4);
                assert!(response.lease.execution_expires_at_ms.unwrap() > started);
            }
        };
        tokio::join!(renew, refresh);
    }
    app.check_storage().await.unwrap();
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn refresh_checks_current_authority_foreign_bases_and_disabled_targets() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, a, b, ak, bk, signer_a, signer_b) = fixture().await;
    let s = app.execution();
    let at = session(&s, network, a, &ak, &signer_a).await;
    let bt = session(&s, network, b, &bk, &signer_b).await;
    let ai = acquire(&app, &at).await;
    let bi = acquire(&app, &bt).await;
    let own = s
        .refresh_configuration(at.token.expose(), query(&ai, bi.target.clone()))
        .await
        .unwrap();
    assert!(own.full.is_some() && own.delta.is_none());
    verify(&own, None, a);
    // A retired derived target stays ungranted on legacy lease-only renewal;
    // explicit refresh reconstructs it from current trusted configuration.
    app.executor.handle().execute(move |db| {
        db.execute("UPDATE node_execution SET target_publication_id=NULL,execution_expires_at=NULL WHERE network_id=?1 AND node_id=?2", rusqlite::params![network.as_uuid().as_bytes().as_slice(), a.as_uuid().as_bytes().as_slice()]).unwrap();
    }).await.unwrap();
    let no_target = s.renew(at.token.expose(), instance(&ai)).await.unwrap();
    assert!(no_target.target.is_none() && no_target.execution_expires_at_ms.is_none());
    let recovered = s
        .refresh_configuration(at.token.expose(), query(&ai, None))
        .await
        .unwrap();
    verify(&recovered, None, a);
    assert!(recovered.lease.execution_expires_at_ms.is_some());
    assert_eq!(
        s.refresh_configuration(at.token.expose(), query(&bi, None))
            .await,
        Err(Error::BindingConflict)
    );
    let mut wrong = query(&ai, None);
    wrong.instance.generation = Version::new(ai.generation.get() + 1).unwrap();
    assert_eq!(
        s.refresh_configuration(at.token.expose(), wrong).await,
        Err(Error::BindingConflict)
    );
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
                ops: vec![ConfigurationOperation::SetNodeSettings {
                    node_id: b,
                    fields: NodeSettingsPatch {
                        enabled: Input::Value(false),
                        ..Default::default()
                    },
                }],
            },
            rid(),
        )
        .await
        .unwrap();
    let disabled = s
        .refresh_configuration(bt.token.expose(), query(&bi, None))
        .await
        .unwrap();
    assert!(!verify(&disabled, None, b).configuration.enabled);
    assert_eq!(disabled.lease.execution_expires_at_ms, None);
    // Real user-node enrollment: current membership is checked even while its
    // session, keys and instance are all otherwise valid.
    let signer = ed25519_dalek::SigningKey::from_bytes(&[93; 32]);
    let user = enroll_user_node(&app, owner.token.expose(), network, &signer, [94; 32]).await;
    let ut = session(&s, network, user.node_id, &user.keys, &signer).await;
    let ui = acquire(&app, &ut).await;
    s.refresh_configuration(ut.token.expose(), query(&ui, None))
        .await
        .unwrap();
    app.executor
        .handle()
        .execute(move |db| {
            db.execute(
                "UPDATE memberships SET revoked_at=?1 WHERE network_id=?2",
                rusqlite::params![
                    store::execution::now().unwrap(),
                    network.as_uuid().as_bytes().as_slice()
                ],
            )
            .unwrap();
            db.execute(
                "UPDATE node_keys SET revoked_at=created_at WHERE node_id=?1 AND purpose='control'",
                [a.as_uuid().as_bytes().as_slice()],
            )
            .unwrap();
        })
        .await
        .unwrap();
    assert_eq!(
        s.refresh_configuration(ut.token.expose(), query(&ui, None))
            .await,
        Err(Error::InvalidCredentials)
    );
    assert_eq!(
        s.refresh_configuration(at.token.expose(), query(&ai, None))
            .await,
        Err(Error::InvalidCredentials)
    );
    app.shutdown().await.unwrap();
}

#[test]
fn refresh_route_reuses_configuration_contract_and_node_session_security() {
    let spec = serde_json::to_value(crate::http::openapi()).unwrap();
    let route = &spec["paths"]["/agent-api/v1/configuration/refresh"]["post"];
    assert_eq!(route["operationId"], "refreshNodeConfiguration");
    assert_eq!(
        route["security"],
        serde_json::json!([{ "nodeSession": [] }])
    );
    assert_eq!(
        route["requestBody"]["content"]["application/json"]["schema"]["$ref"],
        "#/components/schemas/NodeConfigurationRequest"
    );
    assert_eq!(
        route["responses"]["200"]["content"]["application/json"]["schema"]["$ref"],
        "#/components/schemas/NodeConfigurationResponse"
    );
}

#[tokio::test]
async fn execution_window_outlives_individual_permits_without_extending_business_authority() {
    use meshway_policy::admission::{AdmissionError, Traffic, admit_forward};
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, a, b, ak, bk, signer_a, signer_b) = fixture().await;
    let s = app.execution();
    let at = session(&s, network, a, &ak, &signer_a).await;
    let bt = session(&s, network, b, &bk, &signer_b).await;
    let ai = acquire(&app, &at).await;
    let bi = acquire(&app, &bt).await;
    let short = DemandRequest {
        instance: instance(&ai),
        request_id: rid(),
        target: BusinessTarget::Mesh { node: b },
        protocol: BusinessProtocol::Udp,
        port: Some(53.try_into().unwrap()),
        profile: "mesh".into(),
        duration_seconds: 5,
    };
    let long = DemandRequest {
        request_id: rid(),
        protocol: BusinessProtocol::Tcp,
        port: Some(443.try_into().unwrap()),
        duration_seconds: 60,
        ..short.clone()
    };
    s.demand(at.token.expose(), short.clone()).await.unwrap();
    s.demand(at.token.expose(), long.clone()).await.unwrap();
    let granted = s
        .refresh_configuration(bt.token.expose(), query(&bi, None))
        .await
        .unwrap();
    let old = verify(&granted, None, b);
    let short_permit = &old.configuration.permits[&short.request_id];
    let old_long = old.configuration.permits[&long.request_id].clone();
    let expires = short_permit.expires_at_ms;
    assert!(granted.lease.execution_expires_at_ms.unwrap() > expires);
    let traffic = Traffic {
        source: old.configuration.peers[&a].addresses.ipv4.unwrap().into(),
        destination: old.configuration.addresses.ipv4.unwrap().into(),
        protocol: short.protocol,
        source_port: Some(12345.try_into().unwrap()),
        destination_port: short.port,
    };
    let path = &short_permit.paths[0].nodes;
    assert!(
        admit_forward(
            &old.configuration,
            short.request_id,
            path,
            a,
            Some(a),
            &traffic,
            expires - 1
        )
        .is_ok()
    );
    // At the exact business deadline, the Node lease is still live. The exact
    // production predicate nevertheless rejects the old UDP permission.
    assert_eq!(
        admit_forward(
            &old.configuration,
            short.request_id,
            path,
            a,
            Some(a),
            &traffic,
            expires
        ),
        Err(AdmissionError::Expired)
    );
    let long_traffic = Traffic {
        protocol: long.protocol,
        destination_port: long.port,
        ..traffic
    };
    assert!(
        admit_forward(
            &old.configuration,
            long.request_id,
            &old_long.paths[0].nodes,
            a,
            Some(a),
            &long_traffic,
            expires
        )
        .is_ok()
    );
    let short_id = short.request_id;
    app.executor
        .handle()
        .execute(move |db| {
            db.execute(
                "UPDATE execution_demands SET expires_at=0 WHERE id=?1",
                [short_id.as_uuid().as_bytes().as_slice()],
            )
            .unwrap();
        })
        .await
        .unwrap();
    s.renew_demand(
        at.token.expose(),
        RenewDemandRequest {
            instance: instance(&ai),
            request_id: long.request_id,
            expires_at_ms: ai.expires_at_ms,
        },
    )
    .await
    .unwrap();
    let response = s
        .refresh_configuration(bt.token.expose(), query(&bi, granted.lease.target))
        .await
        .unwrap();
    let current = verify(&response, Some(&old), b);
    assert!(
        !current
            .configuration
            .permits
            .contains_key(&short.request_id)
    );
    assert_eq!(
        current.configuration.permits[&long.request_id].paths,
        old_long.paths
    );
    assert_eq!(
        current.configuration.permits[&long.request_id].expires_at_ms,
        ai.expires_at_ms
    );
    assert_eq!(
        admit_forward(
            &current.configuration,
            short.request_id,
            path,
            a,
            Some(a),
            &traffic,
            expires
        ),
        Err(AdmissionError::Permit)
    );
    assert!(
        admit_forward(
            &current.configuration,
            long.request_id,
            &old_long.paths[0].nodes,
            a,
            Some(a),
            &long_traffic,
            expires
        )
        .is_ok()
    );
    let read = s
        .configuration(bt.token.expose(), query(&bi, response.lease.target.clone()))
        .await
        .unwrap();
    assert_eq!(read.lease, response.lease);
    assert!(read.full.is_none() && read.delta.is_none());
    // Current-address retirement remains an execution barrier, despite the
    // independently longer Node window and still-live TCP permit.
    let address = current.configuration.addresses.ipv4.unwrap().to_string();
    let revision = current.configuration.revision;
    app.executor.handle().execute(move |db| {
        db.execute("INSERT INTO address_retirements(network_id,address,previous_node_id,removed_revision,release_after) VALUES(?1,?2,?3,?4,NULL)", rusqlite::params![network.as_uuid().as_bytes().as_slice(), address, b.as_uuid().as_bytes().as_slice(), revision.get() as i64]).unwrap();
    }).await.unwrap();
    let blocked = s
        .refresh_configuration(bt.token.expose(), query(&bi, None))
        .await
        .unwrap();
    verify(&blocked, None, b);
    assert_eq!(blocked.lease.execution_expires_at_ms, None);
    app.shutdown().await.unwrap();
}
