use super::*;
use meshway_configuration::execution::{DnsEgress, DnsEgressUnavailable};

#[tokio::test]
async fn server_exit_dns_tracks_ready_and_withdrawn_capability() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, a, b, ak, bk, asigner, bsigner) = fixture().await;
    let owner = app
        .accounts()
        .login(
            "owner@example.com".into(),
            Secret::new("a sufficiently long password".into()),
            rid(),
        )
        .await
        .unwrap();
    let current = app
        .configurations()
        .read(owner.token.expose(), network, None)
        .await
        .unwrap();
    let mut cfg = current.document.configuration.clone();
    cfg.nodes.get_mut(&b).unwrap().exit_enabled = true;
    cfg.access = vec![AccessRule {
        remark: String::new(),
        src: vec![format!("node:{a}")],
        dst: vec!["10.20.0.0/16".into()],
        ip: vec!["tcp:443".into()],
        bidirectional: false,
    }];
    let profile = "office-exit".to_owned();
    cfg.routing.egress.profiles.insert(
        profile.clone(),
        RoutingProfile {
            rules: vec![EgressRule {
                remark: String::new(),
                matcher: RouteMatch {
                    ip_cidr: vec!["10.20.0.0/16".into()],
                    ..Default::default()
                },
                action: RuleAction::Exit {
                    nodes: vec![format!("node:{b}")],
                    via: vec![],
                },
            }],
            fallback: RouteAction::Unmanaged,
        },
    );
    cfg.routing.egress.assignments = vec![ProfileAssignment {
        target: vec![format!("node:{a}")],
        allowed: vec![profile.clone()],
        default: profile,
        locked: false,
    }];
    app.configurations()
        .apply(
            owner.token.expose(),
            network,
            meshway_protocol::configuration::ConfigurationRequest::Document {
                expected_revision: current.revision,
                request_id: rid(),
                source_jsonc: WireSecret::new(canonical_json(&cfg).unwrap().expose().into()),
            },
            rid(),
        )
        .await
        .unwrap();
    let service = app.execution();
    let at = session(&service, network, a, &ak, &asigner).await;
    let bt = session(&service, network, b, &bk, &bsigner).await;
    let ai = service
        .acquire(
            at.token.expose(),
            AcquireInstanceRequest {
                instance_id: iid(),
                capabilities: caps(),
            },
        )
        .await
        .unwrap();
    let bi = service
        .acquire(
            bt.token.expose(),
            AcquireInstanceRequest {
                instance_id: iid(),
                capabilities: caps(),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        dns(&app, at.token.expose(), &ai).await,
        DnsEgress::Unavailable {
            reason: DnsEgressUnavailable::ExitUnavailable
        }
    );
    report_current(&app, bt.token.expose(), &bi, 1, ExitStatus::Ready).await;
    let DnsEgress::ViaExit {
        exit,
        service: permit,
        generation,
    } = dns(&app, at.token.expose(), &ai).await
    else {
        panic!("ready single exit must compile DNS");
    };
    assert_eq!(exit, b);
    assert_eq!(permit.source, a);
    assert_eq!(permit.source_instance_id, ai.instance_id);
    assert_eq!(permit.generation, generation);
    assert_eq!(permit.path.nodes.first(), Some(&a));
    assert_eq!(permit.path.nodes.last(), Some(&b));
    let DnsEgress::ServeExitResolver { permit: accepted } = dns(&app, bt.token.expose(), &bi).await
    else {
        panic!();
    };
    assert!(accepted.iter().any(|p| p.source == a && p.exit == b));
    report_current(
        &app,
        bt.token.expose(),
        &bi,
        2,
        ExitStatus::Unavailable {
            reason: ExitUnavailableReason::DnsUnavailable,
        },
    )
    .await;
    assert_eq!(
        dns(&app, at.token.expose(), &ai).await,
        DnsEgress::Unavailable {
            reason: DnsEgressUnavailable::ExitUnavailable
        }
    );
    assert_eq!(dns(&app, bt.token.expose(), &bi).await, DnsEgress::NoExit);
    app.shutdown().await.unwrap();
}

async fn dns(app: &App, token: &str, lease: &InstanceLease) -> DnsEgress {
    app.execution()
        .refresh_configuration(
            token,
            NodeConfigurationRequest {
                instance: InstanceRequest {
                    instance_id: lease.instance_id,
                    generation: lease.generation,
                },
                base: None,
            },
        )
        .await
        .unwrap()
        .full
        .unwrap()
        .configuration
        .dns
        .egress
}
