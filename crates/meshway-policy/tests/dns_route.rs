//! Policy binding tests use real compiler output, not request-shaped route grants.
use meshway_configuration::{execution::*, *};
use meshway_model::{
    BoundNodeKey, KeyAlgorithm, NodeId, NodePublicKey, RequestId, Revision, Version,
};
use meshway_policy::{
    SubjectContext,
    compile::{CompilationInput, compile},
    dns_route::{ResolverRouteError, local_resolver_route},
};
use std::{collections::BTreeMap, net::IpAddr};

fn compiled(
    resolver: IpAddr,
    transport: DnsTransport,
    direct: bool,
) -> (NodeConfiguration, RequestId) {
    let source: NodeId = "00000000-0000-4000-8000-000000000001".parse().unwrap();
    let id: RequestId = "00000000-0000-4001-8000-000000000001".parse().unwrap();
    let mut cfg = new_network(
        AddressPools {
            ipv4: vec!["100.64.0.0/24".into()],
            ipv6: vec![],
        },
        "mesh.example".into(),
    )
    .unwrap();
    cfg.tags_owners.insert("tag:test".into(), vec![]);
    cfg.nodes.insert(
        source,
        NodeSettings {
            hostname: "source".into(),
            remark: String::new(),
            identity: NodeIdentity::Tag {
                tags: vec!["tag:test".into()],
            },
            enabled: true,
            addresses: NodeAddresses {
                ipv4: Some("100.64.0.1".parse().unwrap()),
                ipv6: None,
            },
            listeners: BTreeMap::new(),
            dialers: Input::Omitted,
            exit_enabled: false,
        },
    );
    cfg.dns.servers.insert(
        "resolver".into(),
        DnsServer {
            transport,
            server: resolver,
            server_port: 53.try_into().unwrap(),
            use_with_exit: false,
        },
    );
    cfg.dns.default = vec!["resolver".into()];
    cfg.dns.override_local = true;
    if direct {
        cfg.routing.egress.rules.push(EgressRule {
            remark: String::new(),
            matcher: RouteMatch {
                ip_cidr: vec!["0.0.0.0/0".into(), "::/0".into()],
                ..Default::default()
            },
            action: RuleAction::Direct,
        });
    }
    let key = |algorithm| BoundNodeKey {
        id: "00000000-0000-4002-8000-000000000001".parse().unwrap(),
        generation: Version::MIN,
        public_key: NodePublicKey {
            algorithm,
            bytes: [1; 32].into(),
        },
    };
    let inventory = BTreeMap::from([(
        source,
        TrustedNode {
            control_key: key(KeyAlgorithm::Ed25519),
            dataplane_key: key(KeyAlgorithm::X25519),
            capabilities: BTreeMap::new(),
        },
    )]);
    let demands = BTreeMap::from([(
        id,
        Demand {
            id,
            source,
            target: BusinessTarget::Ip {
                address: ExecutionAddress(resolver),
            },
            protocol: match transport {
                DnsTransport::Udp => BusinessProtocol::Udp,
                DnsTransport::Tcp => BusinessProtocol::Tcp,
            },
            port: Some(53.try_into().unwrap()),
            profile: "mesh".into(),
            expires_at_ms: 10_000,
        },
    )]);
    let mut publication = compile(CompilationInput {
        network: "00000000-0000-4003-8000-000000000001".parse().unwrap(),
        revision: Revision::MIN,
        publication: "00000000-0000-4004-8000-000000000001".parse().unwrap(),
        configuration: &cfg,
        subjects: &SubjectContext::default(),
        inventory: &inventory,
        demands: &demands,
        dns: &[],
        rule_sets: &BTreeMap::new(),
        selected_profiles: &BTreeMap::new(),
        now_ms: 100,
    })
    .unwrap();
    (publication.nodes.remove(&source).unwrap().configuration, id)
}

#[test]
fn accepts_only_live_exact_published_resolver_binding_for_both_families_and_transports() {
    for address in ["192.0.2.53", "2001:db8::53"] {
        for transport in [DnsTransport::Udp, DnsTransport::Tcp] {
            for direct in [false, true] {
                let (cfg, id) = compiled(address.parse().unwrap(), transport, direct);
                assert!(local_resolver_route(&cfg, "resolver", id, 100).is_ok());
                assert_eq!(
                    local_resolver_route(&cfg, "resolver", id, 10_000).unwrap_err(),
                    ResolverRouteError::Expired
                );
                assert_eq!(
                    local_resolver_route(&cfg, "missing", id, 100).unwrap_err(),
                    ResolverRouteError::Missing
                );
            }
        }
    }
}

#[test]
fn rejects_same_cidr_endpoint_rebinding_bare_decisions_and_every_changed_grant_dimension() {
    for case in 0..10 {
        let (mut cfg, id) = compiled("192.0.2.53".parse().unwrap(), DnsTransport::Udp, false);
        match case {
            0 => {
                cfg.dns.settings.servers.get_mut("resolver").unwrap().server =
                    "192.0.2.54".parse().unwrap()
            }
            1 => {
                cfg.local_egress.get_mut(&id).unwrap().source =
                    "00000000-0000-4000-8000-000000000002".parse().unwrap()
            }
            2 => cfg.local_egress.get_mut(&id).unwrap().protocol = BusinessProtocol::Tcp,
            3 => cfg.local_egress.get_mut(&id).unwrap().port = Some(5353.try_into().unwrap()),
            4 => cfg.local_egress.get_mut(&id).unwrap().profile = "global:elsewhere".into(),
            5 => cfg.local_egress.get_mut(&id).unwrap().action = LocalEgressAction::Direct,
            6 => {
                cfg.local_egress.get_mut(&id).unwrap().target =
                    BusinessTarget::Mesh { node: cfg.node_id }
            }
            7 => {
                cfg.local_egress.clear();
            }
            8 => {
                cfg.decisions.clear();
            }
            9 => {
                cfg.local_egress.get_mut(&id).unwrap().demand_id =
                    "00000000-0000-4001-8000-000000000002".parse().unwrap()
            }
            _ => unreachable!(),
        }
        assert!(
            local_resolver_route(&cfg, "resolver", id, 100).is_err(),
            "case {case}"
        );
    }
}
