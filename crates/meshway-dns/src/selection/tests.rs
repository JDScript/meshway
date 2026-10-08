use super::*;
use crate::decode_query;
use hickory_proto::op::{MessageType, OpCode, Query as Question};
use meshway_configuration::{DnsConfiguration, DnsTransport, execution::DnsEgress};

fn query(name: &str) -> Query {
    let mut message = Message::new(1, MessageType::Query, OpCode::Query);
    message.add_query(Question::query(
        Name::from_ascii(name).unwrap(),
        RecordType::A,
    ));
    decode_query(&message.to_vec().unwrap()).unwrap()
}

#[test]
fn magic_split_retained_default_and_exit_selection() {
    let server = |use_with_exit| DnsServer {
        transport: DnsTransport::Udp,
        server: "192.0.2.53".parse().unwrap(),
        server_port: 53.try_into().unwrap(),
        use_with_exit,
    };
    let execution = DnsExecution {
        settings: DnsConfiguration {
            magic_dns: true,
            domain: "mesh.example".into(),
            servers: BTreeMap::from([
                ("local".into(), server(false)),
                ("retained".into(), server(true)),
            ]),
            routes: BTreeMap::from([
                ("corp.example".into(), vec!["retained".into()]),
                ("private.example".into(), vec!["local".into()]),
            ]),
            default: vec!["local".into()],
            override_local: false,
            search_domains: vec![],
        },
        mesh_hosts: BTreeMap::from([(
            "known.mesh.example".into(),
            NodeAddresses {
                ipv4: Some("100.64.0.1".parse().unwrap()),
                ipv6: None,
            },
        )]),
        associations: vec![],
        profile: "exit".into(),
        egress: DnsEgress::NoExit,
    };
    let mut dns = PublishedDns::from_execution(&execution).unwrap();
    // Selection projection only, not a fabricated execution snapshot or grant.
    dns.executable_exit = true;
    assert!(matches!(
        dns.select(&query("known.mesh.example."), ExitSelection::Exit),
        Selection::Published(_)
    ));
    assert!(matches!(
        dns.select(&query("unknown.mesh.example."), ExitSelection::Exit),
        Selection::Unavailable(SelectionUnavailable::DiscoveryRequired)
    ));
    for name in ["www.example.", "private.example.", "evilcorp.example."] {
        assert!(
            matches!(
                dns.select(&query(name), ExitSelection::Exit),
                Selection::ExitResolver
            ),
            "{name}"
        );
    }
    let question = query("host.corp.example.");
    let Selection::Resolvers(candidates) = dns.select(&question, ExitSelection::Exit) else {
        panic!();
    };
    assert_eq!(candidates[0].key, "retained");
    dns.default = vec!["retained".into()];
    assert!(matches!(
        dns.select(&query("www.example."), ExitSelection::Exit),
        Selection::Resolvers(_)
    ));
    assert!(matches!(
        dns.select(&query("www.example."), ExitSelection::NoExit),
        Selection::Unmanaged
    ));
    dns.default = vec!["local".into()];
    dns.executable_exit = false;
    assert!(matches!(
        dns.select(&query("www.example."), ExitSelection::Exit),
        Selection::Unavailable(SelectionUnavailable::MissingExitPlan)
    ));
}
