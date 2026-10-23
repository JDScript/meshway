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
    use ForeignRoute::*;
    assert_eq!(plan.foreign_route("mesh.example."), Conflict);
    assert_eq!(plan.foreign_route("Private.Mesh.Example"), Conflict);
    assert_eq!(plan.foreign_route("notmesh.example"), Unrelated);
    assert_eq!(plan.foreign_route("example"), Unrelated);
    // A pure Mesh plan has no fallback: another link's `~.` or search
    // domain routes nothing it covers.
    assert_eq!(plan.foreign_route("."), Unrelated);
    assert_eq!(plan.foreign_route("tko.oraclevcn.com"), Unrelated);
    cfg.override_local = true;
    let plan = SystemDnsPlan::new(&cfg, &DnsPlan::NoExit).unwrap();
    assert_eq!(plan.foreign_route("."), Conflict);
    assert_eq!(plan.foreign_route("a.mesh.example"), Conflict);
}

/// D5: a plan whose fallback (`~.`) covers a name yields it to another
/// link's specific domain (resolved's most specific match), reported as a
/// local domain; its explicit domains and its `~.` still conflict.
#[test]
fn a_fallback_yields_to_specific_foreign_domains_and_explicit_domains_do_not() {
    use ForeignRoute::*;
    let mut cfg = settings();
    cfg.routes
        .insert("corp.example".into(), vec!["10.0.0.53".into()]);
    for plan in [
        SystemDnsPlan::required(
            &cfg,
            &DnsPlan::Unavailable {
                reason: DnsEgressUnavailable::ExitPlanRequired,
            },
        )
        .unwrap(),
        SystemDnsPlan::new(
            &DnsConfiguration {
                override_local: true,
                ..cfg.clone()
            },
            &DnsPlan::NoExit,
        )
        .unwrap(),
    ] {
        // OCI's DHCP search domain on enp0s6 (jp-tyo).
        assert_eq!(plan.foreign_route("tko.oraclevcn.com"), Local);
        assert_eq!(plan.foreign_route("Example."), Local);
        // An explicit domain covers the name: equal or more specific.
        assert_eq!(plan.foreign_route("corp.example"), Conflict);
        assert_eq!(plan.foreign_route("a.corp.example."), Conflict);
        assert_eq!(plan.foreign_route("mesh.example"), Conflict);
        assert_eq!(plan.foreign_route("node.mesh.example"), Conflict);
        // Less specific than an explicit domain: only the fallback's names.
        assert_eq!(plan.foreign_route("notcorp.example"), Local);
        // A second `~.` competes with the fallback.
        assert_eq!(plan.foreign_route("."), Conflict);
        assert_eq!(plan.foreign_route(""), Conflict);
    }
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
        provenance: crate::system::Provenance::Original,
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

/// RC-1 M6 (G3): the runtime reference's two columns, as the owner judges
/// them. Proven (stays blocked, read again slowly): the anchor changed or
/// ended on a conflict, foreign link settings, an interrupted settings
/// change, another owner's reservation, another namespace. Anything a read
/// could not settle, or that recovers by itself, is unknown.
#[test]
fn failures_are_proven_only_when_read_back_different() {
    use super::{SystemDnsError::*, SystemDnsFailure};
    for check in [
        "namespace",
        "owner_record",
        "link_settings",
        "record_pending",
        "record_plan",
        "anchor_link",
        "anchor_identity",
        "anchor_address_collision",
        "anchor_ended",
    ] {
        assert!(SystemDnsFailure::new(Conflict, check).proven(), "{check}");
    }
    for (error, check) in [
        (Backend, "anchor_ended"),
        (Backend, "observe_deadline"),
        (Backend, "anchor_netlink"),
        (Conflict, "foreign_domain"),
        (Conflict, "resolved_owner"),
        (Conflict, "resolved_namespace"),
        (Conflict, "owner_fenced"),
        (Backend, "link_settings"),
        (Storage, "record_write"),
        (Conflict, "record_read"),
        (Unsupported, "resolv_conf_mode"),
    ] {
        assert!(
            !SystemDnsFailure::new(error, check).proven(),
            "{error:?} {check}"
        );
    }
    assert!(SystemDnsFailure::new(Backend, "link_settings").system());
    assert!(SystemDnsFailure::new(Unsupported, "resolv_conf_mode").system());
    assert!(!SystemDnsFailure::new(ResolverPlan, "unrecorded").system());
    assert!(!SystemDnsFailure::new(Unsupported, "unrecorded").system());
}
