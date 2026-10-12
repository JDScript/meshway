//! Trusted DNS associations decide the addresses of names, for one source.
use super::*;
use meshway_configuration::netmap::{AssociationRecordType, DnsAssociation, DnsAssociationOverlay};

/// Source 1: `chat.example` and its subdomains exit through 4 via 3; the rest
/// is direct. Access grants only `domain:chat.example` on tcp:443.
fn split() -> Scenario {
    let mut s = super::exit::setup();
    s.cfg.access = vec![access(
        &[sel(1)],
        &["domain:chat.example".into()],
        &["tcp:443"],
        false,
    )];
    let mut rule = egress_rule("192.0.2.0/32", exit_action(&[4], &[3]));
    rule.matcher.ip_cidr.clear();
    rule.matcher.domain_suffix = vec!["chat.example".into()];
    let profile = s.cfg.routing.egress.profiles.get_mut("exit").unwrap();
    profile.rules = vec![rule];
    profile.fallback = RouteAction::Direct;
    s
}

fn association(
    compiled: &CompiledNetmaps,
    name: &str,
    addresses: &[&str],
    expires_at_ms: u64,
) -> DnsAssociation {
    let plan = compiled.nodes[&node(1)].netmap.egress.as_ref().unwrap();
    DnsAssociation {
        source: node(1),
        generation: plan.generation,
        name: name.into(),
        record_type: AssociationRecordType::A,
        cnames: vec![],
        addresses: addresses
            .iter()
            .map(|a| ExecutionAddress(a.parse().unwrap()))
            .collect(),
        expires_at_ms,
    }
}

fn overlay(at: &CompiledNetmap, mut entries: Vec<DnsAssociation>) -> DnsAssociationOverlay {
    entries.sort_by(|a, b| (a.source, &a.name).cmp(&(b.source, &b.name)));
    DnsAssociationOverlay {
        netmap: at.reference(),
        associations: entries,
    }
}

fn decide(
    compiled: &CompiledNetmaps,
    at: u8,
    associations: &Associations,
    destination: &str,
) -> LocalDecision {
    LocalCompiler::new(&compiled.nodes[&node(at)].netmap, &Default::default())
        .decide_with(
            node(1),
            destination.parse().unwrap(),
            BusinessProtocol::Tcp,
            port(443),
            associations,
        )
        .unwrap()
}

#[test]
fn a_name_rule_decides_only_the_addresses_associated_with_the_name() {
    let compiled = netmaps(&split());
    let none = Associations::default();
    assert_eq!(
        decide(&compiled, 1, &none, "203.0.113.9"),
        LocalDecision::Direct
    );
    let plan = compiled.nodes[&node(1)].netmap.egress.as_ref().unwrap();
    assert_eq!(plan.unavailable, None);
    let source = LocalCompiler::new(&compiled.nodes[&node(1)].netmap, &Default::default());
    let base = source.capture().unwrap().unwrap();
    assert!(base.managed_ipv4.is_empty());

    let entries = vec![association(
        &compiled,
        "api.chat.example",
        &["203.0.113.9"],
        u64::MAX,
    )];
    let mut selections = Vec::new();
    for at in [1, 4] {
        let held = Associations::new(
            Some(&overlay(&compiled.nodes[&node(at)], entries.clone())),
            0,
        );
        let LocalDecision::Permitted(decision) = decide(&compiled, at, &held, "203.0.113.9") else {
            panic!("association decides the exit at {at}");
        };
        assert_eq!(
            decision.target,
            Target::Exit {
                nodes: vec![node(4)],
                via: vec![node(3)]
            }
        );
        assert!(!decision.paths.is_empty());
        assert!(
            decision
                .paths
                .iter()
                .all(|path| path.nodes.first() == Some(&node(1))
                    && path.nodes.contains(&node(3))
                    && path.nodes.last() == Some(&node(4)))
        );
        selections.push(decision.selection);
        // Only the associated address: its neighbour stays direct.
        assert_eq!(
            decide(&compiled, at, &held, "203.0.113.10"),
            LocalDecision::Direct
        );
    }
    assert_eq!(selections[0], selections[1]);
    // The relay carries the name exit without any association.
    let relay = LocalCompiler::new(&compiled.nodes[&node(3)].netmap, &Default::default());
    assert!(
        !relay
            .transit(
                node(1),
                &Target::Exit {
                    nodes: vec![node(4)],
                    via: vec![node(3)]
                }
            )
            .unwrap()
            .is_empty()
    );

    // The source captures exactly that host; the rest stays direct.
    let held = Associations::new(Some(&overlay(&compiled.nodes[&node(1)], entries)), 0);
    let (carved, summary) = source.associated_capture(node(1), &base, &held).unwrap();
    let host: std::net::Ipv4Addr = "203.0.113.9".parse().unwrap();
    assert_eq!(carved.managed_ipv4, [(host, host)]);
    assert_eq!(carved.generation, plan.generation);
    assert_eq!(carved.sink_ipv6, base.sink_ipv6);
    // The summary counts what is installed with the association, not the
    // partition without it.
    assert_eq!(
        (summary.managed_ipv4, summary.routes, summary.unavailable),
        (1, 1, None)
    );
    assert_ne!(summary, source.capture_summary(node(1)).unwrap());
}

#[test]
fn incompatible_names_selection_access_and_expiry_never_widen() {
    let compiled = netmaps(&split());
    let at = &compiled.nodes[&node(1)];
    let shared = vec![
        association(&compiled, "api.chat.example", &["203.0.113.9"], u64::MAX),
        association(&compiled, "cdn.other.example", &["203.0.113.9"], u64::MAX),
    ];
    let conflict = Associations::new(Some(&overlay(at, shared.clone())), 0);
    assert_eq!(
        decide(&compiled, 1, &conflict, "203.0.113.9"),
        LocalDecision::Denied(UnavailableReason::EgressConflict)
    );
    // A conflicting address is still captured, so it is dropped, not sent direct.
    let source = LocalCompiler::new(&at.netmap, &Default::default());
    let base = source.capture().unwrap().unwrap();
    let (carved, _) = source
        .associated_capture(node(1), &base, &conflict)
        .unwrap();
    let host: std::net::Ipv4Addr = "203.0.113.9".parse().unwrap();
    assert_eq!(carved.managed_ipv4, [(host, host)]);

    // Another selection identity never reuses the association.
    let mut stale = association(&compiled, "api.chat.example", &["203.0.113.9"], u64::MAX);
    stale.generation = Digest::from_bytes([9; 32]);
    let stale = Associations::new(Some(&overlay(at, vec![stale])), 0);
    assert_eq!(
        decide(&compiled, 1, &stale, "203.0.113.9"),
        LocalDecision::Direct
    );

    // Ended entries are left out.
    let ended = Associations::new(
        Some(&overlay(
            at,
            vec![association(
                &compiled,
                "api.chat.example",
                &["203.0.113.9"],
                10,
            )],
        )),
        10,
    );
    assert!(ended.is_empty());

    // Without a `domain:` (or Internet) grant the exit is unauthorized.
    let mut s = split();
    s.cfg.access = vec![access(
        &[sel(1)],
        &["10.20.0.0/16".into()],
        &["tcp:443"],
        false,
    )];
    let ungranted = netmaps(&s);
    let held = Associations::new(
        Some(&overlay(
            &ungranted.nodes[&node(1)],
            vec![association(
                &ungranted,
                "api.chat.example",
                &["203.0.113.9"],
                u64::MAX,
            )],
        )),
        0,
    );
    assert_eq!(
        decide(&ungranted, 1, &held, "203.0.113.9"),
        LocalDecision::Denied(UnavailableReason::Unauthorized)
    );
}
