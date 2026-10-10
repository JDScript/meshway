use super::*;
use meshway_configuration::execution::DnsEgressUnavailable;

fn settings() -> DnsConfiguration {
    serde_json::from_str(r#"{"domain":"mesh.example"}"#).unwrap()
}

#[test]
fn nullable_owner_records_preserve_object_shape_and_strict_nested_decoding() {
    for backend in [
        SystemDnsBackend::Resolved,
        SystemDnsBackend::OwnedResolvConf {
            path: "/etc/resolv.conf".into(),
        },
    ] {
        let settings = SystemDnsSettings {
            backend,
            ..SystemDnsSettings::default()
        };
        let bytes = serde_json::to_vec(&Some(settings.clone())).unwrap();
        assert_eq!(
            decode_optional_record::<SystemDnsSettings>(&bytes).unwrap(),
            Some(settings)
        );
    }
    assert_eq!(
        decode_optional_record::<SystemDnsSettings>(b"null").unwrap(),
        None
    );
    for bytes in [
        br#"["127.0.0.1:5354",{"backend":"resolved"},"198.18.255.254"]"#.as_slice(),
        br#"{"bind":"127.0.0.1:5354","backend":{"backend":"resolved","backend":"resolved"},"anchorAddress":"198.18.255.254"}"#.as_slice(),
        br#"{"bind":"127.0.0.1:5354","backend":{"backend":"resolved"},"anchorAddress":"198.18.255.254","extra":true}"#.as_slice(),
        b"null null".as_slice(),
    ] {
        assert_eq!(decode_optional_record::<SystemDnsSettings>(bytes), Err(SystemDnsError::Storage));
    }
}

#[test]
fn routing_conflicts_are_label_bounded_and_include_foreign_more_specific_routes() {
    let mut cfg = settings();
    cfg.domain = "mesh.example".into();
    let plan = SystemDnsPlan::new(&cfg, &DnsPlan::NoExit).unwrap();
    assert_eq!(
        plan.domains().first(),
        Some(&("mesh.example".into(), false))
    );
    assert!(plan.conflicts("mesh.example."));
    assert!(plan.conflicts("Private.Mesh.Example"));
    assert!(!plan.conflicts("notmesh.example"));
    assert!(!plan.conflicts("example"));
    assert!(!plan.conflicts("."));
    cfg.override_local = true;
    let plan = SystemDnsPlan::new(&cfg, &DnsPlan::NoExit).unwrap();
    assert!(plan.conflicts("."));
    assert!(plan.conflicts("ordinary.example"));
}

#[test]
fn published_egress_proof_and_search_routing_are_not_inferred() {
    let mut cfg = settings();
    assert_eq!(
        SystemDnsPlan::new(
            &cfg,
            &DnsPlan::Unavailable {
                reason: DnsEgressUnavailable::ExitPlanRequired
            }
        )
        .unwrap_err(),
        SystemDnsError::ExitPlan
    );
    let negative = SystemDnsPlan::required(
        &cfg,
        &DnsPlan::Unavailable {
            reason: DnsEgressUnavailable::ExitPlanRequired,
        },
    )
    .unwrap();
    assert!(negative.domains().contains(&(".".into(), true)));
    cfg.domain = "mesh.example".into();
    cfg.search_domains = vec!["mesh.example".into()];
    assert!(
        !SystemDnsPlan::new(&cfg, &DnsPlan::NoExit)
            .unwrap()
            .needs_system_fallback(&cfg)
    );
    cfg.search_domains.push("ordinary.example".into());
    assert!(
        SystemDnsPlan::new(&cfg, &DnsPlan::NoExit)
            .unwrap()
            .needs_system_fallback(&cfg)
    );
    assert_eq!(
        SystemDnsPlan::new(&cfg, &DnsPlan::NoExit)
            .unwrap()
            .domains(),
        vec![
            ("mesh.example".into(), false),
            ("ordinary.example".into(), false)
        ]
    );
}

#[tokio::test]
async fn stale_system_provenance_wakes_blocked_work_and_never_renews_itself() {
    let snapshot = SystemResolverSnapshot {
        id: "fixture-provenance-only".into(),
        upstreams: Arc::new(vec![]),
        primary: DnsTransport::Udp,
        invalid: CancellationToken::new(),
        verified_until: Arc::new(Mutex::new(
            Instant::now() + std::time::Duration::from_millis(20),
        )),
    };
    assert!(snapshot.ensure_current().is_ok());
    tokio::time::timeout(std::time::Duration::from_secs(1), snapshot.ended())
        .await
        .unwrap();
    assert!(snapshot.ensure_current().is_err());
}

#[test]
fn explicit_file_proxy_requires_loopback_port_53_and_an_absolute_nontraversing_path() {
    let mut cfg = SystemDnsSettings {
        backend: SystemDnsBackend::OwnedResolvConf {
            path: "/etc/resolv.conf".into(),
        },
        ..Default::default()
    };
    cfg.bind.set_port(5354);
    assert_eq!(cfg.validate(), Err(SystemDnsError::Settings));
    cfg.bind.set_port(53);
    assert!(cfg.validate().is_ok());
    cfg.backend = SystemDnsBackend::OwnedResolvConf {
        path: "/etc/../etc/resolv.conf".into(),
    };
    assert_eq!(cfg.validate(), Err(SystemDnsError::Settings));
}
