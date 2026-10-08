use super::*;

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
