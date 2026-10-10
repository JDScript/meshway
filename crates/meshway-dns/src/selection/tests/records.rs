use crate::{
    ConfigurationError, ExitSelection, PublishedDns, ResponseTransport, Selection,
    SelectionUnavailable, decode_query, encode_response,
};
use hickory_proto::{
    op::{Message, MessageType, OpCode, Query as Question},
    rr::{Name, RData, RecordType},
};
use meshway_configuration::{DnsConfiguration, DnsServer, DnsTransport, NodeAddresses};
use std::collections::BTreeMap;
fn query_message(name: &str, kind: RecordType) -> Message {
    let mut message = Message::new(0x1234, MessageType::Query, OpCode::Query);
    message.metadata.recursion_desired = true;
    message.add_query(Question::query(Name::from_ascii(name).unwrap(), kind));
    message
}

struct Names {
    settings: DnsConfiguration,
    mesh_hosts: BTreeMap<String, NodeAddresses>,
}

fn published(source: &Names) -> Result<PublishedDns, ConfigurationError> {
    PublishedDns::from_parts(&source.settings, &source.mesh_hosts, false)
}

fn execution() -> Names {
    Names {
        settings: DnsConfiguration {
            magic_dns: true,
            domain: "mesh.example".into(),
            servers: BTreeMap::from([
                (
                    "public".into(),
                    DnsServer {
                        transport: DnsTransport::Udp,
                        server: "192.0.2.53".parse().unwrap(),
                        server_port: 53.try_into().unwrap(),
                        use_with_exit: false,
                    },
                ),
                (
                    "office".into(),
                    DnsServer {
                        transport: DnsTransport::Tcp,
                        server: "10.0.0.53".parse().unwrap(),
                        server_port: 5353.try_into().unwrap(),
                        use_with_exit: true,
                    },
                ),
            ]),
            default: vec!["public".into()],
            routes: BTreeMap::from([
                ("example".into(), vec!["public".into()]),
                (
                    "corp.example".into(),
                    vec!["office".into(), "public".into()],
                ),
                ("private.corp.example".into(), vec!["public".into()]),
                ("mesh.example".into(), vec!["public".into()]),
            ]),
            search_domains: vec![],
            override_local: false,
        },
        mesh_hosts: BTreeMap::from([
            (
                "node.mesh.example".into(),
                NodeAddresses {
                    ipv4: Some("100.64.0.1".parse().unwrap()),
                    ipv6: Some("fd00::1".parse().unwrap()),
                },
            ),
            (
                "v4.mesh.example".into(),
                NodeAddresses {
                    ipv4: Some("100.64.0.2".parse().unwrap()),
                    ipv6: None,
                },
            ),
        ]),
    }
}

fn resolver_keys(snapshot: &PublishedDns, name: &str, exit: ExitSelection) -> Vec<String> {
    let query = decode_query(&query_message(name, RecordType::A).to_vec().unwrap()).unwrap();
    match snapshot.select(&query, exit) {
        Selection::Resolvers(candidates) => candidates.iter().map(|c| c.key.to_owned()).collect(),
        other => panic!("expected resolver candidates, got {other:?}"),
    }
}

#[test]
fn published_records_precede_split_and_preserve_question_case_and_explicit_ttl() {
    let snapshot = published(&execution()).unwrap();
    for exit in [ExitSelection::NoExit, ExitSelection::Exit] {
        for kind in [RecordType::A, RecordType::AAAA] {
            let query =
                decode_query(&query_message("NoDe.MeSh.ExAmPlE.", kind).to_vec().unwrap()).unwrap();
            let Selection::Published(answer) = snapshot.select(&query, exit) else {
                panic!("published host");
            };
            for ttl in [0, 37] {
                let bytes =
                    encode_response(&query, &answer.response(ttl), ResponseTransport::Udp, 1232)
                        .unwrap();
                let message = Message::from_vec(&bytes).unwrap();
                assert_eq!(message.metadata.id, query.id());
                assert_eq!(message.queries[0].name().to_ascii(), "NoDe.MeSh.ExAmPlE.");
                assert_eq!(message.answers.len(), 1);
                assert_eq!(message.answers[0].ttl, ttl);
                assert_eq!(message.answers[0].name.to_ascii(), "NoDe.MeSh.ExAmPlE.");
                match &message.answers[0].data {
                    RData::A(address) => assert_eq!(address.0.to_string(), "100.64.0.1"),
                    RData::AAAA(address) => assert_eq!(address.0.to_string(), "fd00::1"),
                    other => panic!("unexpected record: {other:?}"),
                }
                assert!(!message.metadata.authoritative);
                assert!(!message.metadata.authentic_data);
            }
        }
    }
}

#[test]
fn null_family_is_nodata_but_missing_name_requires_discovery() {
    let snapshot = published(&execution()).unwrap();
    let query = decode_query(
        &query_message("v4.mesh.example.", RecordType::AAAA)
            .to_vec()
            .unwrap(),
    )
    .unwrap();
    let Selection::Published(answer) = snapshot.select(&query, ExitSelection::NoExit) else {
        panic!("known host");
    };
    let bytes =
        encode_response(&query, &answer.response(10), ResponseTransport::Udp, 1232).unwrap();
    let message = Message::from_vec(&bytes).unwrap();
    assert_eq!(
        message.metadata.response_code,
        hickory_proto::op::ResponseCode::NoError
    );
    assert!(message.answers.is_empty());
    assert!(message.authorities.is_empty());
    for name in ["missing.mesh.example.", "mesh.example."] {
        let query = decode_query(&query_message(name, RecordType::A).to_vec().unwrap()).unwrap();
        assert!(matches!(
            snapshot.select(&query, ExitSelection::NoExit),
            Selection::Unavailable(SelectionUnavailable::DiscoveryRequired)
        ));
    }
    let query = decode_query(
        &query_message("v4.mesh.example.", RecordType::TXT)
            .to_vec()
            .unwrap(),
    )
    .unwrap();
    assert!(matches!(
        snapshot.select(&query, ExitSelection::NoExit),
        Selection::Unavailable(SelectionUnavailable::UnsupportedRecordType)
    ));
}

#[test]
fn split_selection_is_longest_label_suffix_and_preserves_resolver_order() {
    let snapshot = published(&execution()).unwrap();
    assert_eq!(
        resolver_keys(&snapshot, "API.CORP.EXAMPLE.", ExitSelection::NoExit),
        ["office", "public"]
    );
    assert_eq!(
        resolver_keys(
            &snapshot,
            "api.private.corp.example.",
            ExitSelection::NoExit
        ),
        ["public"]
    );
    assert_eq!(
        resolver_keys(&snapshot, "evilcorp.example.", ExitSelection::NoExit),
        ["public"]
    );
    assert_eq!(
        resolver_keys(
            &snapshot,
            "_service._tcp.corp.example.",
            ExitSelection::NoExit
        ),
        ["office", "public"]
    );
    // A literal dot inside one wire label cannot create another matching label.
    let mut message = query_message("placeholder.example.", RecordType::A);
    message.queries[0].name =
        Name::from_labels([b"api.corp".as_slice(), b"example".as_slice()]).unwrap();
    let query = decode_query(&message.to_vec().unwrap()).unwrap();
    let Selection::Resolvers(candidates) = snapshot.select(&query, ExitSelection::NoExit) else {
        panic!("parent route");
    };
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].key, "public");
}

#[test]
fn no_exit_override_and_exit_exceptions_are_explicit_selection_only() {
    let mut source = execution();
    let snapshot = published(&source).unwrap();
    let ordinary = decode_query(
        &query_message("outside.test.", RecordType::A)
            .to_vec()
            .unwrap(),
    )
    .unwrap();
    assert!(matches!(
        snapshot.select(&ordinary, ExitSelection::NoExit),
        Selection::Unmanaged
    ));
    assert!(matches!(
        snapshot.select(&ordinary, ExitSelection::Exit),
        Selection::Unavailable(SelectionUnavailable::MissingExitPlan)
    ));
    assert_eq!(
        resolver_keys(&snapshot, "api.corp.example.", ExitSelection::Exit),
        ["office"]
    );
    // The unretained narrower route drops out; the kept parent still applies.
    assert_eq!(
        resolver_keys(&snapshot, "api.private.corp.example.", ExitSelection::Exit),
        ["office"]
    );
    let query = decode_query(
        &query_message("api.corp.example.", RecordType::A)
            .to_vec()
            .unwrap(),
    )
    .unwrap();
    let Selection::Resolvers(candidates) = snapshot.select(&query, ExitSelection::Exit) else {
        panic!("explicit resolver");
    };
    assert_eq!(candidates[0].server.transport, DnsTransport::Tcp);
    assert_eq!(candidates[0].server.server.to_string(), "10.0.0.53");
    assert_eq!(candidates[0].server.server_port.get(), 5353);
    assert!(candidates[0].server.use_with_exit);

    source.settings.override_local = true;
    let snapshot = published(&source).unwrap();
    assert_eq!(
        resolver_keys(&snapshot, "outside.test.", ExitSelection::NoExit),
        ["public"]
    );
    assert!(matches!(
        snapshot.select(&ordinary, ExitSelection::Exit),
        Selection::Unavailable(SelectionUnavailable::MissingExitPlan)
    ));
    source.settings.override_local = false;
    source.settings.default.push("office".into());
    let snapshot = published(&source).unwrap();
    assert_eq!(
        resolver_keys(&snapshot, "outside.test.", ExitSelection::Exit),
        ["office"]
    );
    assert!(matches!(
        snapshot.select(&ordinary, ExitSelection::NoExit),
        Selection::Unmanaged
    ));
}

#[test]
fn magic_dns_disable_and_snapshot_replacement_are_explicit() {
    let mut source = execution();
    let original = published(&source).unwrap();
    source.settings.magic_dns = false;
    let disabled = published(&source).unwrap();
    assert_eq!(
        resolver_keys(&disabled, "node.mesh.example.", ExitSelection::NoExit),
        ["public"]
    );
    let query = decode_query(
        &query_message("node.mesh.example.", RecordType::A)
            .to_vec()
            .unwrap(),
    )
    .unwrap();
    assert!(matches!(
        original.select(&query, ExitSelection::NoExit),
        Selection::Published(_)
    ));
    source.settings.magic_dns = true;
    source.mesh_hosts.clear();
    let replaced = published(&source).unwrap();
    assert!(matches!(
        replaced.select(&query, ExitSelection::NoExit),
        Selection::Unavailable(SelectionUnavailable::DiscoveryRequired)
    ));
}

#[test]
fn settings_reuse_idna_normalization_and_reject_ambiguous_inputs() {
    let mut source = execution();
    source
        .settings
        .routes
        .insert("BÜCHER.test.".into(), vec!["office".into()]);
    let snapshot = published(&source).unwrap();
    assert_eq!(
        resolver_keys(&snapshot, "api.xn--bcher-kva.test.", ExitSelection::NoExit),
        ["office"]
    );
    source
        .settings
        .routes
        .insert("xn--bcher-kva.test".into(), vec!["public".into()]);
    assert!(matches!(
        published(&source),
        Err(ConfigurationError::DuplicateName)
    ));
    let mut source = execution();
    source.settings.default.push("unknown".into());
    assert!(matches!(
        published(&source),
        Err(ConfigurationError::ResolverReference)
    ));
    let mut source = execution();
    source.settings.override_local = true;
    source.settings.default.clear();
    assert!(matches!(
        published(&source),
        Err(ConfigurationError::ResolverReference)
    ));
    let mut source = execution();
    source.mesh_hosts.insert(
        "other.test".into(),
        NodeAddresses {
            ipv4: None,
            ipv6: None,
        },
    );
    assert!(matches!(
        published(&source),
        Err(ConfigurationError::HostOutsideDomain)
    ));
    let mut source = execution();
    source.mesh_hosts.insert(
        "NODE.MESH.EXAMPLE.".into(),
        NodeAddresses {
            ipv4: None,
            ipv6: None,
        },
    );
    assert!(matches!(
        published(&source),
        Err(ConfigurationError::DuplicateName)
    ));
}
