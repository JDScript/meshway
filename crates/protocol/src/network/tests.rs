use super::*;

fn node(id: u32, name: &str) -> MeshNode {
    MeshNode {
        id: format!("{id:064x}"),
        name: name.into(),
        country: None,
        mesh_ipv4: format!("100.96.0.{id}"),
        mesh_ipv6: format!("fd00::{id:x}"),
    }
}

fn dns() -> MeshDnsConfiguration {
    MeshDnsConfiguration {
        enabled: true,
        ..Default::default()
    }
}

fn payload() -> NodeDnsConfiguration {
    let dns = dns();
    let (records, diagnostics) =
        compile_dns_records(&dns, &[node(1, "one"), node(2, "two")]).unwrap();
    NodeDnsConfiguration {
        schema_version: 1,
        revision: 1,
        node_id: format!("{:064x}", 1),
        network_id: "a".repeat(32),
        dns,
        records,
        diagnostics,
    }
}

#[test]
fn zones_normalize_idna_and_reject_reserved_ips_urls_and_bad_bounds() {
    let mut config = dns();
    config.zone = "BÜCHER.Test.".into();
    assert_eq!(config.normalized().unwrap().zone, "xn--bcher-kva.test");
    for zone in [
        "",
        ".",
        "test",
        "localhost",
        "a.localhost",
        "127.0.0.1",
        "::1",
        "in-addr.arpa",
        "1.in-addr.arpa",
        "ip6.arpa",
        "a.ip6.arpa",
        "*.test",
        "https://test.example",
        "foo..test",
        "-foo.test",
        "foo-.test",
        "foo.test/path",
        "foo.test:53",
    ] {
        config.zone = zone.into();
        assert!(config.normalized().is_err(), "{zone}");
    }
    config.zone = format!("{}.{}.test", "a".repeat(63), "b".repeat(61));
    assert!(config.normalized().is_err());
    config.zone = "meshway.test".into();
    for ttl in [0, 301, u32::MAX] {
        config.ttl_seconds = ttl;
        assert!(config.normalized().is_err());
    }
    for ttl in [1, 300] {
        config.ttl_seconds = ttl;
        assert!(config.normalized().is_ok());
    }
}

#[test]
fn aliases_are_network_unique_without_overwriting_display_names_or_identity() {
    let nodes = vec![
        node(7, "BÜCHER"),
        node(1, "One"),
        node(2, "one"),
        node(3, "not a label"),
        node(4, "a.outside.test"),
        node(5, "id"),
        node(6, "localhost"),
    ];
    let (records, diagnostics) = compile_dns_records(&dns(), &nodes).unwrap();
    assert_eq!(
        records.iter().map(|r| &r.node_id).collect::<Vec<_>>(),
        (1..=7)
            .map(|id| format!("{id:064x}"))
            .collect::<Vec<_>>()
            .iter()
            .collect::<Vec<_>>()
    );
    for record in &records[..6] {
        assert!(record.aliases.is_empty());
    }
    assert_eq!(records[6].aliases, ["xn--bcher-kva.meshway.test"]);
    assert_eq!(diagnostics.len(), 6);
    assert_eq!(nodes[1].name, "One");
    assert_eq!(
        records[0].canonical_name,
        format!("id.{}.{:032x}.meshway.test", "0".repeat(32), 1)
    );
    let unicode_collision = [node(1, "BÜCHER"), node(2, "xn--bcher-kva")];
    assert!(
        compile_dns_records(&dns(), &unicode_collision)
            .unwrap()
            .0
            .iter()
            .all(|r| r.aliases.is_empty())
    );
    assert!(compile_dns_records(&dns(), &[node(1, "x"), node(1, "y")]).is_err());
    let mut invalid = node(1, "x");
    invalid.id = "root".into();
    assert!(compile_dns_records(&dns(), &[invalid]).is_err());
}

#[test]
fn snapshot_validation_bounds_metadata_names_and_canonical_digest() {
    let source = payload();
    source.validate().unwrap();
    assert_eq!(
        dns_digest(&source).unwrap(),
        dns_digest(
            &serde_json::from_slice::<NodeDnsConfiguration>(&canonical_json(&source).unwrap())
                .unwrap()
        )
        .unwrap()
    );
    let mut invalid = source.clone();
    invalid.schema_version = 2;
    assert!(invalid.validate().is_err());
    let mut invalid = source.clone();
    invalid.revision = 0;
    assert!(invalid.validate().is_err());
    let mut invalid = source.clone();
    invalid.network_id = "A".repeat(32);
    assert!(invalid.validate().is_err());
    let mut invalid = source.clone();
    invalid.node_id = "root".into();
    assert!(invalid.validate().is_err());
    let mut invalid = source.clone();
    invalid.records[0].canonical_name = "outside.test".into();
    assert!(invalid.validate().is_err());
    let mut invalid = source.clone();
    invalid.records[0].canonical_name.push('.');
    assert!(invalid.validate().is_err());
    let mut invalid = source.clone();
    invalid.records[0].aliases = vec!["outside.test".into()];
    assert!(invalid.validate().is_err());
    let mut invalid = source.clone();
    invalid.records[0].aliases = vec!["id.meshway.test".into()];
    assert!(invalid.validate().is_err());
    let mut invalid = source.clone();
    invalid.records[0].aliases = invalid.records[1].aliases.clone();
    assert!(invalid.validate().is_err());
    let mut invalid = source.clone();
    invalid.records.swap(0, 1);
    assert!(invalid.validate().is_err());
    let mut invalid = source.clone();
    invalid.records[0].mesh_ipv4 = "not-an-ip".into();
    assert!(invalid.validate().is_err());
    let mut invalid = source.clone();
    invalid.dns.enabled = false;
    assert!(invalid.validate().is_err());
    invalid.records.clear();
    invalid.validate().unwrap();
    let mut invalid = source;
    invalid.diagnostics.push(PolicyDiagnostic {
        rule_id: None,
        field: "dns".into(),
        message: "x".repeat(MAX_DNS_BYTES),
    });
    assert!(invalid.validate().is_err());
    assert!(
        serde_json::from_value::<SaveNetworkRequest>(serde_json::json!({"expected_revision":1,
        "name":"Meshway","dns":dns(),"ipv4_prefix":"100.97.0.0/16"}))
        .is_err()
    );
}

#[test]
fn record_and_diagnostic_budgets_are_enforced() {
    let nodes: Vec<_> = (1..=MAX_DNS_RECORDS)
        .map(|id| {
            let mut node = node(1, "invalid label");
            node.id = format!("{id:064x}");
            node.mesh_ipv4 = Ipv4Addr::from(
                u32::from(Ipv4Addr::new(100, 96, 0, 0)) + u32::try_from(id).unwrap(),
            )
            .to_string();
            node.mesh_ipv6 = Ipv6Addr::from(
                u128::from("fd00::".parse::<Ipv6Addr>().unwrap()) + u128::try_from(id).unwrap(),
            )
            .to_string();
            node
        })
        .collect();
    let (records, diagnostics) = compile_dns_records(&dns(), &nodes).unwrap();
    assert_eq!(records.len(), MAX_DNS_RECORDS);
    assert_eq!(diagnostics.len(), MAX_DNS_DIAGNOSTICS);
    let mut too_many = nodes;
    too_many.push(node(1025, "extra"));
    assert!(compile_dns_records(&dns(), &too_many).is_err());
    let disabled = MeshDnsConfiguration::default();
    assert!(
        compile_dns_records(&disabled, &[node(1, "one")])
            .unwrap()
            .0
            .is_empty()
    );
}

fn nodes_from_records(records: &[NodeDnsRecord]) -> Vec<MeshNode> {
    records
        .iter()
        .enumerate()
        .map(|(index, record)| MeshNode {
            id: record.node_id.clone(),
            name: format!("node-{index}"),
            country: None,
            mesh_ipv4: record.mesh_ipv4.clone(),
            mesh_ipv6: record.mesh_ipv6.clone(),
        })
        .collect()
}

fn assert_validator_and_compiler_reject(source: &NodeDnsConfiguration) {
    // Recomputing a valid digest for a malformed bundle is not semantic validation.
    let snapshot = AgentDnsSnapshot {
        digest: dns_digest(source).unwrap(),
        payload: source.clone(),
    };
    let restored: AgentDnsSnapshot =
        serde_json::from_slice(&canonical_json(&snapshot).unwrap()).unwrap();
    assert_eq!(restored.digest, dns_digest(&restored.payload).unwrap());
    assert!(restored.payload.validate().is_err());
    assert!(
        compile_dns_records(
            &restored.payload.dns,
            &nodes_from_records(&restored.payload.records)
        )
        .is_err()
    );
}

#[test]
fn validator_and_compiler_require_unique_address_owners_by_value() {
    let source = payload();
    for ipv6 in [false, true] {
        let mut invalid = source.clone();
        if ipv6 {
            invalid.records[1].mesh_ipv6 = invalid.records[0].mesh_ipv6.clone();
        } else {
            invalid.records[1].mesh_ipv4 = invalid.records[0].mesh_ipv4.clone();
        }
        assert_validator_and_compiler_reject(&invalid);
    }
    for equivalent in [
        "fd00:0:0:0:0:0:0:1",
        "FD00:0000:0000:0000:0000:0000:0000:0001",
    ] {
        let mut invalid = source.clone();
        invalid.records[1].mesh_ipv6 = equivalent.into();
        assert_validator_and_compiler_reject(&invalid);
    }
    // A different spelling alone is legal when the parsed owner stays unique.
    let mut valid = source;
    valid.records[0].mesh_ipv6 = "FD00:0000:0000:0000:0000:0000:0000:0001".into();
    valid.validate().unwrap();
    compile_dns_records(&valid.dns, &nodes_from_records(&valid.records)).unwrap();
}

#[test]
fn validator_and_compiler_reject_non_endpoint_addresses() {
    let source = payload();
    for ipv4 in [
        "not-an-ip",
        "0.0.0.0",
        "224.0.0.1",
        "239.255.255.255",
        "255.255.255.255",
        "127.0.0.1",
        "127.255.255.254",
        "169.254.0.1",
        "169.254.255.254",
    ] {
        let mut invalid = source.clone();
        invalid.records[0].mesh_ipv4 = ipv4.into();
        assert_validator_and_compiler_reject(&invalid);
    }
    for ipv6 in [
        "not-an-ip",
        "::",
        "0:0:0:0:0:0:0:0",
        "ff02::1",
        "ff0e::1",
        "::1",
        "fe80::1",
        "febf::1",
        "::ffff:100.96.0.1",
        "0:0:0:0:0:ffff:6460:1",
    ] {
        let mut invalid = source.clone();
        invalid.records[0].mesh_ipv6 = ipv6.into();
        assert_validator_and_compiler_reject(&invalid);
    }
}

#[test]
fn enabled_bundles_require_exactly_one_self_and_disabled_empty_remains_valid() {
    let source = payload();
    // MeshNode is active public inventory without credentials or public keys;
    // those keyless nodes remain valid DNS owners.
    let nodes = nodes_from_records(&source.records);
    assert_eq!(compile_dns_records(&source.dns, &nodes).unwrap().0.len(), 2);
    source.validate().unwrap();
    let mut absent = source.clone();
    absent.node_id = "f".repeat(64);
    assert!(absent.validate().is_err());
    assert_eq!(dns_digest(&absent).unwrap().len(), 64);
    let mut empty = source.clone();
    empty.records.clear();
    assert!(empty.validate().is_err());
    let mut duplicate = source.clone();
    duplicate.records.insert(0, duplicate.records[0].clone());
    assert_validator_and_compiler_reject(&duplicate);
    // Neither disabled configuration nor compiler publication needs a self
    // record, matching the existing withdrawal contract.
    empty.dns.enabled = false;
    empty.validate().unwrap();
    assert!(
        compile_dns_records(&empty.dns, &nodes)
            .unwrap()
            .0
            .is_empty()
    );
    absent.dns.enabled = false;
    absent.records.clear();
    absent.validate().unwrap();
}
