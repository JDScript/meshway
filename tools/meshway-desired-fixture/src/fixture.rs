//! Deterministic topology and valid IP/TCP wire inputs; keys and IDs are random.
use crate::{Error, NodeMaterial, StaticAuthority, random_uuid};
use meshway_configuration::transport::PreparedListener;
use meshway_configuration::{execution::*, *};
use meshway_model::{
    state::{Capability, StreamKind, TransportRole},
    *,
};
use meshway_policy::SubjectContext;
use std::{collections::BTreeMap, net::Ipv4Addr};

pub const LISTENER_KEY: &str = "perfudp";

pub struct Chain {
    pub authority: StaticAuthority,
    pub path: Vec<NodeId>,
    /// The (origin = path[0], target) pair every chain circuit is admitted under.
    pub target: meshway_policy::compile::netmap::Target,
    pub carriers: Vec<Carrier>,
    pub addresses: Vec<Ipv4Addr>,
}

impl Chain {
    pub fn new(nodes: usize) -> Result<Self, Error> {
        if !(2..=8).contains(&nodes) {
            return Err("chain requires 2..=8 nodes".into());
        }
        let mut configuration = new_network(
            AddressPools {
                ipv4: vec!["100.64.0.0/24".into()],
                ipv6: vec![],
            },
            "perf.internal".into(),
        )?;
        configuration.tags_owners.insert("tag:perf".into(), vec![]);
        let mut path = Vec::new();
        let mut addresses = Vec::new();
        let mut materials = BTreeMap::new();
        for i in 0..nodes {
            let id = NodeId::new(random_uuid()?)?;
            let address = Ipv4Addr::new(100, 64, 0, u8::try_from(i + 1)?);
            let listener = serde_json::from_str(r#"{"type":"native_udp","listenPort":23456}"#)?;
            configuration.nodes.insert(
                id,
                NodeSettings {
                    hostname: format!("n{i}"),
                    remark: String::new(),
                    identity: NodeIdentity::Tag {
                        tags: vec!["tag:perf".into()],
                    },
                    enabled: true,
                    addresses: NodeAddresses {
                        ipv4: Some(address),
                        ipv6: None,
                    },
                    listeners: BTreeMap::from([(
                        LISTENER_KEY.into(),
                        PreparedListener::prepare(listener)?,
                    )]),
                    dialers: Input::Omitted,
                    exit_enabled: false,
                    region: None,
                },
            );
            materials.insert(
                id,
                NodeMaterial::generate(BTreeMap::from([(
                    "transport:native_udp".into(),
                    Capability::Transport {
                        roles: vec![TransportRole::Dial, TransportRole::Listen],
                        streams: vec![StreamKind::Udp],
                    },
                )]))?,
            );
            path.push(id);
            addresses.push(address);
        }
        let last = path[nodes - 1];
        configuration.access = vec![AccessRule {
            remark: String::new(),
            src: vec![format!("node:{}", path[0])],
            dst: vec![format!("node:{last}")],
            ip: vec!["tcp:443".into()],
            bidirectional: false,
        }];
        configuration.routing.mesh = vec![MeshRoute {
            remark: String::new(),
            src: vec![format!("node:{}", path[0])],
            dst: vec![format!("node:{last}")],
            via: path[1..nodes - 1]
                .iter()
                .map(|id| format!("node:{id}"))
                .collect(),
        }];
        let authority = StaticAuthority::compile(
            NetworkId::new(random_uuid()?)?,
            &configuration,
            &SubjectContext::default(),
            materials,
        )?;
        crate::check_path(
            authority.netmap(path[0])?,
            path[0],
            std::net::IpAddr::V4(addresses[nodes - 1]),
            (BusinessProtocol::Tcp, 443),
            &path,
        )?;
        let carriers = path
            .windows(2)
            .map(|pair| Carrier {
                dialer: pair[0],
                listener: pair[1],
                listener_key: LISTENER_KEY.into(),
                protocol: TransportProtocol::NativeUdp,
            })
            .collect();
        Ok(Self {
            authority,
            path,
            target: meshway_policy::compile::netmap::Target::Mesh { node: last },
            carriers,
            addresses,
        })
    }

    pub fn carriers_for(&self, index: usize) -> Result<BTreeMap<NodeId, Carrier>, Error> {
        if index >= self.path.len() {
            return Err("unknown chain index".into());
        }
        let mut result = BTreeMap::new();
        if index > 0 {
            result.insert(self.path[index - 1], self.carriers[index - 1].clone());
        }
        if index + 1 < self.path.len() {
            result.insert(self.path[index + 1], self.carriers[index].clone());
        }
        Ok(result)
    }
}

/// 20-byte IPv4 + 32-byte TCP (12 NOP options) + deterministic payload. Thus
/// 52 bytes is a pure ACK and 1280 bytes carries 1228 bytes of TCP data.
pub fn tcp_ipv4(
    source: Ipv4Addr,
    destination: Ipv4Addr,
    reply: bool,
    sequence: u32,
    acknowledgement: u32,
    flags: u8,
    length: usize,
) -> Result<Vec<u8>, Error> {
    if !(52..=65_519).contains(&length) {
        return Err("packet length outside 52..=65519".into());
    }
    let mut bytes = vec![0x5a; length];
    bytes[..52].fill(0);
    bytes[0] = 0x45;
    bytes[2..4].copy_from_slice(&u16::try_from(length)?.to_be_bytes());
    bytes[8] = 64;
    bytes[9] = 6;
    bytes[12..16].copy_from_slice(&source.octets());
    bytes[16..20].copy_from_slice(&destination.octets());
    let (source_port, destination_port): (u16, u16) =
        if reply { (443, 49152) } else { (49152, 443) };
    bytes[20..22].copy_from_slice(&source_port.to_be_bytes());
    bytes[22..24].copy_from_slice(&destination_port.to_be_bytes());
    bytes[24..28].copy_from_slice(&sequence.to_be_bytes());
    bytes[28..32].copy_from_slice(&acknowledgement.to_be_bytes());
    bytes[32] = 8 << 4;
    bytes[33] = flags;
    bytes[34..36].copy_from_slice(&65535u16.to_be_bytes());
    bytes[40..52].fill(1);
    let check = checksum(&bytes[..20]);
    bytes[10..12].copy_from_slice(&check.to_be_bytes());
    let mut pseudo = Vec::with_capacity(length);
    pseudo.extend_from_slice(&bytes[12..20]);
    pseudo.extend_from_slice(&[0, 6]);
    pseudo.extend_from_slice(&u16::try_from(length - 20)?.to_be_bytes());
    pseudo.extend_from_slice(&bytes[20..]);
    let check = checksum(&pseudo);
    bytes[36..38].copy_from_slice(&check.to_be_bytes());
    Ok(bytes)
}

fn checksum(bytes: &[u8]) -> u16 {
    let mut sum = bytes.chunks(2).fold(0u64, |sum, word| {
        sum + (u64::from(word[0]) << 8) + u64::from(*word.get(1).unwrap_or(&0))
    });
    while sum >> 16 != 0 {
        sum = (sum & 65535) + (sum >> 16);
    }
    !(sum as u16)
}
