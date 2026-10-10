use super::*;
use crate::resources::content_digest;
use meshway_model::state::*;
use std::net::IpAddr;

fn id<T: std::str::FromStr>(n: u128) -> T
where
    T::Err: std::fmt::Debug,
{
    uuid::Uuid::from_u128(n).to_string().parse().unwrap()
}
pub(super) fn setup() -> (ReportAdmission, StateReport, VisibilityScope, DateTime<Utc>) {
    let now = Utc::now();
    let node = id(1);
    let peer = id(2);
    let reference = NetmapRef {
        revision: Revision::MIN,
        serial: meshway_model::Version::MIN,
        digest: content_digest(b"node configuration"),
    };
    let resource = ResourceRef::Listener { key: "udp".into() };
    let admission = ReportAdmission {
        network_id: id(4),
        node_id: node,
        instance_id: id(5),
        instance_generation: Version::MIN,
        lease_expires_at: now + Duration::minutes(5),
        execution: Some(reference),
        configurations: vec![reference],
        contexts: vec![ConfigurationAdmission {
            configuration: reference,
            installation_allowed: true,
            resources: BTreeMap::from([(
                resource.clone(),
                ResourceAdmission {
                    network: SocketNetwork::Udp,
                    transport: "native_udp".into(),
                    configured_endpoints: BTreeMap::from([(
                        "/listeners/udp/endpoints/0".into(),
                        "1.1.1.1:9999".into(),
                    )]),
                },
            )]),
            peers: BTreeSet::from([peer]),
            neighbor_transports: BTreeMap::from([(peer, BTreeSet::from(["native_udp".into()]))]),
            mesh_address_pools: meshway_configuration::AddressPools {
                ipv4: vec!["100.64.0.0/10".into()],
                ipv6: vec![],
            },
            profiles: BTreeSet::from(["mesh".into()]),
        }],
        mesh_addresses: BTreeSet::from(["100.64.0.1".parse::<IpAddr>().unwrap()]),
    };
    let report = StateReport {
        instance_id: admission.instance_id,
        sequence: Sequence::new(1).unwrap(),
        observed_at: now,
        state: NodeState {
            exit_capability: None,
            system: SystemState {
                hostname: "host".into(),
                os: "linux".into(),
                arch: "arm64".into(),
                agent_version: "test".into(),
            },
            capabilities: BTreeMap::new(),
            configuration: ConfigurationState {
                saved: Some(reference),
                applied: Some(reference),
                progress: None,
            },
            listeners: BTreeMap::from([(
                "udp".into(),
                ListenerState {
                    phase: ListenerPhase::Listening,
                    socket_ids: vec!["s1".into()],
                    loaded_certificate_sha256: None,
                    problem: None,
                    netmap: None,
                },
            )]),
            sockets: BTreeMap::from([(
                "s1".into(),
                SocketState {
                    local_endpoints: None,
                    resource,
                    netmap: None,
                    generation: Sequence::new(1).unwrap(),
                    network: SocketNetwork::Udp,
                    bound_address: "0.0.0.0:9999".parse().unwrap(),
                    endpoints: vec![EndpointCandidate {
                        address: "1.1.1.1:9999".into(),
                        origin: EndpointSource::Stun {
                            observer: "8.8.8.8:3478".into(),
                        },
                        observed_at: now,
                        expires_at: now + Duration::seconds(10),
                    }],
                },
            )]),
            links: BTreeMap::new(),
            adjacencies: Default::default(),
            paths: BTreeMap::new(),
            selection: SelectionState {
                applied_profile: Some("mesh".into()),
            },
            probes: vec![],
            problems: vec![],
        },
    };
    let scope = VisibilityScope {
        network_id: admission.network_id,
        configuration_revision: Revision::MIN,
        authorization_digest: content_digest(b"authorization"),
        observation_authorization_digest: content_digest(b"authorization"),
        visible: BTreeSet::from([node, peer]),
        hostnames: BTreeMap::from([(node, "node".into()), (peer, "peer".into())]),
        full: BTreeSet::from([node]),
        endpoint_resources: None,
        mesh_addresses: admission.mesh_addresses.clone(),
        expires_at: now + Duration::minutes(10),
        current: BTreeMap::from([(
            node,
            CurrentNode {
                instance_id: Some(admission.instance_id),
                instance_generation: Some(admission.instance_generation),
                execution: Some(reference),
                configurations: vec![reference],
                revoked: false,
                lease_expires_at: Some(admission.lease_expires_at),
            },
        )]),
    };
    (admission, report, scope, now)
}

pub(super) fn identity(network: NetworkId) -> service::ReadIdentity {
    service::ReadIdentity::UserSession {
        digest: [42; 32],
        network,
    }
}

mod admission;
