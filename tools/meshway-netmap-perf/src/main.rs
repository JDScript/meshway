//! Pure projection measurement; no authority issuance, live State or packets.
#![forbid(unsafe_code)]
use clap::Parser;
use meshway_configuration::{execution::TrustedNode, netmap::*, *};
use meshway_model::{
    state::{Capability, StreamKind, TransportRole},
    *,
};
use meshway_policy::compile::netmap::*;
use meshway_transport::spec::PreparedListener;
use serde::Serialize;
use std::{collections::BTreeMap, time::Instant};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Parser)]
struct Options {
    #[command(flatten)]
    logging: meshway_logging::Options,
    #[arg(long, default_value_t = 1000)]
    nodes: u16,
    #[arg(long, default_value_t = 10)]
    group_size: u16,
    #[arg(long, default_value_t = 3)]
    samples: u16,
}

struct Scenario {
    configuration: NetworkConfiguration,
    subjects: SubjectContext,
    inventory: BTreeMap<NodeId, TrustedNode>,
    previous: BTreeMap<NodeId, PreviousNetmap>,
    selected: BTreeMap<NodeId, String>,
    versions: BTreeMap<NodeId, Version>,
    rule_sets: meshway_policy::compile::RuleSets,
}

fn node(n: u16) -> NodeId {
    NodeId::new(uuid::Uuid::from_u128(u128::from(n) + 1)).unwrap()
}

impl Scenario {
    fn new(count: u16, group_size: u16) -> Result<Self> {
        let mut configuration = new_network(
            AddressPools {
                ipv4: vec!["100.64.0.0/16".into()],
                ipv6: vec![],
            },
            "perf.internal".into(),
        )?;
        configuration.defaults.topology = TopologyDefault::Deny;
        configuration.access.clear();
        let mut inventory = BTreeMap::new();
        for n in 0..count {
            let tag = format!("tag:group-{}", n / group_size);
            if !configuration.tags_owners.contains_key(&tag) {
                configuration.tags_owners.insert(tag.clone(), vec![]);
                configuration.topology.push(TopologyRule {
                    remark: String::new(),
                    action: TopologyAction::Allow,
                    a: tag.clone(),
                    b: tag.clone(),
                    transports: Input::Omitted,
                    keepalive: false,
                });
                configuration.access.push(AccessRule {
                    remark: String::new(),
                    src: vec![tag.clone()],
                    dst: vec![tag.clone()],
                    ip: vec!["*".into()],
                    bidirectional: false,
                });
            }
            let listener = serde_json::from_str(r#"{"type":"native_udp","listenPort":23456}"#)?;
            configuration.nodes.insert(
                node(n),
                NodeSettings {
                    hostname: format!("node-{n}"),
                    remark: String::new(),
                    identity: NodeIdentity::Tag { tags: vec![tag] },
                    enabled: true,
                    addresses: NodeAddresses {
                        ipv4: Some(std::net::Ipv4Addr::from(
                            u32::from(std::net::Ipv4Addr::new(100, 64, 0, 1)) + u32::from(n),
                        )),
                        ipv6: None,
                    },
                    listeners: BTreeMap::from([(
                        "udp".into(),
                        PreparedListener::prepare(listener)?,
                    )]),
                    dialers: Input::Omitted,
                    exit_enabled: false,
                    region: None,
                },
            );
            let key = |offset, algorithm| BoundNodeKey {
                id: KeyId::new(uuid::Uuid::from_u128(u128::from(n) + offset)).unwrap(),
                generation: Version::MIN,
                public_key: NodePublicKey {
                    algorithm,
                    bytes: [1; 32].into(),
                },
            };
            inventory.insert(
                node(n),
                TrustedNode {
                    control_key: key(1, KeyAlgorithm::Ed25519),
                    dataplane_key: key(20_001, KeyAlgorithm::X25519),
                    capabilities: BTreeMap::from([(
                        "transport:native_udp".into(),
                        Capability::Transport {
                            roles: vec![TransportRole::Dial, TransportRole::Listen],
                            streams: vec![StreamKind::Udp],
                        },
                    )]),
                },
            );
        }
        Ok(Self {
            configuration,
            inventory,
            subjects: Default::default(),
            previous: BTreeMap::new(),
            selected: BTreeMap::new(),
            versions: BTreeMap::new(),
            rule_sets: BTreeMap::new(),
        })
    }

    fn input(&self, revision: Revision) -> NetmapInput<'_> {
        NetmapInput {
            network: NetworkId::new(uuid::Uuid::from_u128(1)).unwrap(),
            revision,
            configuration: &self.configuration,
            subjects: &self.subjects,
            inventory: &self.inventory,
            rule_sets: &self.rule_sets,
            selected_profiles: &self.selected,
            selection_versions: &self.versions,
            previous: &self.previous,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Measurement {
    scenario: &'static str,
    evaluated_nodes: usize,
    full_evaluated_nodes: usize,
    incremental_ms: Vec<f64>,
    full_ms: Vec<f64>,
    incremental_median_ms: f64,
    full_median_ms: f64,
    speedup: f64,
    equivalent: bool,
}

fn median(values: &[f64]) -> f64 {
    let mut ordered = values.to_vec();
    ordered.sort_by(f64::total_cmp);
    ordered[ordered.len() / 2]
}

fn measure(
    s: &Scenario,
    base: &IncrementalNetmaps,
    samples: u16,
    name: &'static str,
) -> Result<Measurement> {
    let mut incremental_ms = Vec::new();
    let mut full_ms = Vec::new();
    let mut evaluated = None;
    for _ in 0..samples {
        let start = Instant::now();
        let incremental = compile_incremental(s.input(Revision::new(2)?), &base.inputs)?;
        incremental_ms.push(start.elapsed().as_secs_f64() * 1000.0);
        evaluated = Some(incremental.compiled.nodes.len());
        let start = Instant::now();
        let full = compile_netmap(s.input(Revision::new(2)?))?;
        full_ms.push(start.elapsed().as_secs_f64() * 1000.0);
        let mut merged = base.compiled.nodes.clone();
        merged.retain(|node, _| incremental.inputs.contains_key(node));
        merged.extend(incremental.compiled.nodes);
        if merged != full.nodes {
            return Err("incremental/full output mismatch".into());
        }
    }
    let (inc, full) = (median(&incremental_ms), median(&full_ms));
    Ok(Measurement {
        scenario: name,
        evaluated_nodes: evaluated.ok_or("no samples")?,
        full_evaluated_nodes: s.inventory.len(),
        incremental_ms,
        full_ms,
        incremental_median_ms: inc,
        full_median_ms: full,
        speedup: full / inc,
        equivalent: true,
    })
}

fn main() -> Result<()> {
    let options = Options::parse();
    options.logging.init()?;
    if !(2..=10_000).contains(&options.nodes) || options.group_size < 2 || options.samples == 0 {
        return Err("require 2..10000 nodes, group size >= 2 and nonzero samples".into());
    }
    let mut s = Scenario::new(options.nodes, options.group_size)?;
    let base = compile_incremental(s.input(Revision::MIN), &BTreeMap::new())?;
    s.previous = base
        .compiled
        .nodes
        .iter()
        .map(|(id, netmap)| (*id, PreviousNetmap::from(netmap)))
        .collect();
    let mut measurements = vec![measure(&s, &base, options.samples, "revision-only")?];
    s.configuration.nodes.get_mut(&node(0)).unwrap().hostname = "renamed".into();
    measurements.push(measure(&s, &base, options.samples, "one-node-hostname")?);
    // A Node-triggered event: one Node's own capabilities change (rolling
    // upgrade, exit role), as `acquire` reports them.
    s.inventory
        .get_mut(&node(1))
        .ok_or("node 1")?
        .capabilities
        .insert(
            "transport:native_tcp".into(),
            Capability::Transport {
                roles: vec![TransportRole::Dial],
                streams: vec![StreamKind::Tcp],
            },
        );
    measurements.push(measure(&s, &base, options.samples, "one-node-capability")?);
    s.configuration.dns.domain = "changed.internal".into();
    measurements.push(measure(&s, &base, options.samples, "global-dns-domain")?);
    let revision = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()?;
    if !revision.status.success() {
        return Err("cannot determine source revision".into());
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "compilerVersion": NETMAP_COMPILER_VERSION, "sourceRevision": String::from_utf8(revision.stdout)?.trim(),
            "nodes": options.nodes, "groupSize": options.group_size, "samples": options.samples,
            "scope": "pure projection including input fingerprinting; excludes SQL, signing and packet traffic",
            "measurements": measurements,
        }))?
    );
    Ok(())
}
