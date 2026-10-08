use super::*;

pub(super) fn mesh(protocol: BusinessProtocol) -> (NetworkConfiguration, Demand, Traffic) {
    let mut configuration = configuration();
    let token = match protocol {
        BusinessProtocol::Tcp => "tcp:443",
        BusinessProtocol::Udp => "udp:443",
        BusinessProtocol::IcmpEcho => "icmp:*",
    };
    configuration.access = vec![AccessRule {
        remark: String::new(),
        src: vec![format!("node:{}", node(1))],
        dst: vec![format!("node:{}", node(2))],
        ip: vec![token.into()],
        bidirectional: false,
    }];
    configuration.routing.mesh = vec![MeshRoute {
        remark: String::new(),
        src: vec![format!("node:{}", node(1))],
        dst: vec![format!("node:{}", node(2))],
        via: vec![format!("node:{}", node(3))],
    }];
    let mut demand = demand(1, 1, 2);
    demand.protocol = protocol;
    if protocol == BusinessProtocol::IcmpEcho {
        demand.port = None;
    }
    let traffic = Traffic {
        source: "100.64.0.1".parse().unwrap(),
        destination: "100.64.0.2".parse().unwrap(),
        protocol,
        source_port: (protocol != BusinessProtocol::IcmpEcho).then(|| Port::new(49152).unwrap()),
        destination_port: demand.port,
    };
    (configuration, demand, traffic)
}

// Uses the same full semantic normalization and compiler entry points as
// Server, then the public Node admission API. No transport/auth stand-ins.
pub(super) fn publication(
    configuration: &NetworkConfiguration,
    demand: &Demand,
    dns: &[TrustedDnsAssociation],
) -> CompiledPublication {
    let context = ValidationContext {
        nodes: configuration.nodes.keys().copied().collect(),
        approved_identities: configuration
            .nodes
            .iter()
            .map(|(id, node)| (*id, node.identity.clone()))
            .collect(),
        ..Default::default()
    };
    let (configuration, _) = meshway_policy::normalize(configuration.into(), &context).unwrap();
    run(
        &configuration,
        &inventory(),
        &BTreeMap::from([(demand.id, demand.clone())]),
        dns,
    )
}

pub(super) fn reverse(forward: Traffic) -> Traffic {
    Traffic {
        source: forward.destination,
        destination: forward.source,
        source_port: forward.destination_port,
        destination_port: forward.source_port,
        ..forward
    }
}

pub(super) fn assign_exit_profile(
    configuration: &mut NetworkConfiguration,
    demand: &mut Demand,
    matcher: RouteMatch,
) {
    let profile = "source-exit";
    configuration.routing.egress.profiles.insert(
        profile.into(),
        RoutingProfile {
            rules: vec![EgressRule {
                remark: String::new(),
                matcher,
                action: RuleAction::Exit {
                    nodes: vec![format!("node:{}", node(4))],
                    via: vec![format!("node:{}", node(3))],
                },
            }],
            fallback: RouteAction::Unmanaged,
        },
    );
    configuration.routing.egress.assignments = vec![ProfileAssignment {
        target: vec![format!("node:{}", demand.source)],
        allowed: vec![profile.into()],
        default: profile.into(),
        locked: true,
    }];
    demand.profile = profile.into();
}
