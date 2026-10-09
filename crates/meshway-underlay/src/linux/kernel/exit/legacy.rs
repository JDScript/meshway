//! Read-only compatibility for retiring X3a's private kernel journal. Old
//! records never become live projections or a new execution authority.
use super::{
    ExitIncarnation, ExitTransport, OriginalTuple,
    conntrack::{Entry, Tuple},
    objects::Receipt,
    program::Program,
    storage::{Ledger, Projection},
};
use crate::{Error, Result};
use meshway_configuration::execution::ExitRolePlan;
use meshway_model::{NetworkId, NodeId};
use serde::Deserialize;
use std::net::Ipv4Addr;

#[derive(Deserialize)]
struct Version {
    version: u32,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct V1 {
    version: u32,
    boot: String,
    namespace_device: u64,
    namespace_inode: u64,
    base_installation: String,
    network: NetworkId,
    node: NodeId,
    plan: ExitRolePlan,
    program: Program,
    tun_name: String,
    mtu: usize,
    route_table: u32,
    rules: Vec<Vec<u8>>,
    routes: Vec<Vec<u8>>,
    receipt: Option<Receipt>,
    projections: Vec<OldProjection>,
    quarantine: Vec<Tuple>,
    closing: bool,
    removed: bool,
    pending: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OldProjection {
    incarnation: u128,
    slot: u32,
    tuple: OldTuple,
    expires_ms: u64,
    retired: bool,
    ct: Option<Entry>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OldTuple {
    source: Ipv4Addr,
    destination: Ipv4Addr,
    transport: OldTransport,
}
#[derive(Deserialize)]
#[serde(tag = "protocol", deny_unknown_fields)]
enum OldTransport {
    Tcp {
        source_port: u16,
        destination_port: u16,
    },
    Udp {
        source_port: u16,
        destination_port: u16,
    },
    IcmpEcho {
        identifier: u16,
    },
}

pub(super) fn decode(bytes: &[u8]) -> Result<Ledger> {
    match serde_json::from_slice::<Version>(bytes)
        .map_err(|_| Error::Journal)?
        .version
    {
        2 => serde_json::from_slice(bytes).map_err(|_| Error::Journal),
        1 => {
            let old: V1 = serde_json::from_slice(bytes).map_err(|_| Error::Journal)?;
            if old.version != 1 {
                return Err(Error::Journal);
            }
            let mut program = old.program;
            program.legacy = true;
            let projections = old
                .projections
                .into_iter()
                .map(|old| {
                    let mut handle = [0; 32];
                    handle[16..].copy_from_slice(&old.incarnation.to_be_bytes());
                    let transport = match old.tuple.transport {
                        OldTransport::Tcp {
                            source_port,
                            destination_port,
                        } => ExitTransport::Tcp {
                            source_port,
                            destination_port,
                        },
                        OldTransport::Udp {
                            source_port,
                            destination_port,
                        } => ExitTransport::Udp {
                            source_port,
                            destination_port,
                        },
                        // This placeholder is used only by exact identifier-based
                        // CT deletion. The broad wire tombstone records that the
                        // old format had no sequence evidence.
                        OldTransport::IcmpEcho { identifier } => ExitTransport::IcmpEcho {
                            identifier,
                            sequence: 0,
                        },
                    };
                    Projection {
                        incarnation: ExitIncarnation::new(handle),
                        slot: old.slot,
                        tuple: OriginalTuple {
                            source: old.tuple.source,
                            destination: old.tuple.destination,
                            transport,
                        },
                        expires_ms: old.expires_ms,
                        retired: old.retired,
                        ct: old.ct,
                    }
                })
                .collect();
            Ok(Ledger {
                version: 2,
                boot: old.boot,
                namespace_device: old.namespace_device,
                namespace_inode: old.namespace_inode,
                base_installation: old.base_installation,
                network: old.network,
                node: old.node,
                plan: (&old.plan).into(),
                program,
                tun_name: old.tun_name,
                mtu: old.mtu,
                route_table: old.route_table,
                rules: old.rules,
                routes: old.routes,
                receipt: old.receipt,
                projections,
                quarantine: Vec::new(),
                legacy_quarantine: old.quarantine,
                closing: old.closing,
                removed: old.removed,
                pending: old.pending,
            })
        }
        _ => Err(Error::Journal),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn v1_cleanup_preserves_all_old_handle_bits_and_unknown_echo_sequence() {
        let bytes = format!(
            r#"{{"version":1,"boot":"fixture","namespace_device":4,"namespace_inode":5,"base_installation":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","network":"11111111-1111-4111-8111-111111111111","node":"22222222-2222-4222-8222-222222222222","plan":{{"instanceId":"33333333-3333-4333-8333-333333333333","generation":"0101010101010101010101010101010101010101010101010101010101010101","family":"ipv4","nat":"masquerade"}},"program":{{"installation":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","interface":2,"interface_name":"tun0","mark":1,"pools":["100.96.0.0/11"]}},"tun_name":"tun0","mtu":1280,"route_table":20000,"rules":[],"routes":[],"receipt":null,"projections":[{{"incarnation":{},"slot":1,"tuple":{{"source":"100.96.0.1","destination":"192.0.2.1","transport":{{"protocol":"IcmpEcho","identifier":77}}}},"expires_ms":1,"retired":false,"ct":null}}],"quarantine":[{{"source":"192.0.2.1","destination":"198.51.100.1","protocol":1,"source_key":77,"destination_key":0}}],"closing":false,"removed":false,"pending":false}}"#,
            u128::MAX
        );
        let old = decode(bytes.as_bytes()).unwrap();
        assert!(old.program.legacy);
        assert_eq!(&old.projections[0].incarnation.bytes()[16..], &[255; 16]);
        assert_eq!(&old.projections[0].incarnation.bytes()[..16], &[0; 16]);
        assert!(old.quarantine.is_empty());
        assert_eq!(old.legacy_quarantine.len(), 1);
        assert_eq!(
            decode(&serde_json::to_vec(&old).unwrap()).unwrap().version,
            2
        );
        assert!(decode(bytes.replace("\"version\":1", "\"version\":3").as_bytes()).is_err());
    }
}
