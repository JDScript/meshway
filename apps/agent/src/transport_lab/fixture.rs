use std::{
    io,
    net::{Ipv4Addr, SocketAddr},
    path::Path,
};

use meshway_dataplane::{
    Identity, LabConfig, Limits, ListenerConfig, OutgoingCandidate, PeerConfig, PrivateKey,
    TransportKind,
};
use meshway_protocol::local;

use super::{
    Result,
    config::{Config, Credential, VlessListener, VlessOutgoing},
    tun::TunConfig,
};

pub fn generate(
    directory: &Path,
    udp: SocketAddr,
    vless: SocketAddr,
    use_vless: bool,
    tun: bool,
) -> Result<()> {
    if udp.port() == 0 || vless.port() == 0 {
        return Err(io::Error::other("fixture endpoints require nonzero ports").into());
    }
    local::private_dir(directory)?;
    let _lock = local::lock(&directory.join("fixture.lock"))?;
    let names = ["nyc", "lax", "sha"];
    for name in names {
        if directory.join(format!("{name}.json")).try_exists()? {
            return Err(
                io::Error::other("fixture already exists; use a fresh private directory").into(),
            );
        }
    }
    let identities = names
        .iter()
        .map(|_| Identity::generate())
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let forward: Vec<String> = names.iter().map(|name| (*name).into()).collect();
    let reverse: Vec<String> = names.iter().rev().map(|name| (*name).into()).collect();
    let random = local::credential()?;
    let uuid = Credential::new(format!(
        "{}-{}-{}-{}-{}",
        &random[..8],
        &random[8..12],
        &random[12..16],
        &random[16..20],
        &random[20..32]
    ))?;
    let target = SocketAddr::from((Ipv4Addr::LOCALHOST, 8421));
    for (index, name) in names.iter().enumerate() {
        let is_lax = *name == "lax";
        let mut listeners = Vec::new();
        let mut outgoing = Vec::new();
        if is_lax || *name == "nyc" || !use_vless {
            listeners.push(ListenerConfig {
                transport: TransportKind::NativeUdp,
                id: None,
                allowed_peers: None,
                bind: SocketAddr::from((
                    Ipv4Addr::UNSPECIFIED,
                    if is_lax { udp.port() } else { 0 },
                )),
            });
            if !is_lax {
                outgoing.push(OutgoingCandidate {
                    id: None,
                    peer: "lax".into(),
                    transport: TransportKind::NativeUdp,
                    endpoint: if *name == "sha" {
                        vless.to_string()
                    } else {
                        udp.to_string()
                    },
                    priority: 10,
                });
            }
        }
        if is_lax && !use_vless && udp.port() != vless.port() {
            listeners.push(ListenerConfig {
                transport: TransportKind::NativeUdp,
                id: None,
                allowed_peers: None,
                bind: SocketAddr::from((Ipv4Addr::UNSPECIFIED, vless.port())),
            });
        }
        let config = Config {
            schema_version: 1,
            dataplane: LabConfig {
                node_id: (*name).into(),
                private_key: PrivateKey::from_hex(&identities[index].private_key_hex())?,
                public_key: identities[index].public_key_hex(),
                trusted_peers: names
                    .iter()
                    .enumerate()
                    .filter(|(other, _)| *other != index)
                    .map(|(other, peer)| PeerConfig {
                        node_id: (*peer).into(),
                        public_key: identities[other].public_key_hex(),
                        transports: if index != 1 && other != 1 {
                            vec![]
                        } else if use_vless && (index == 2 || other == 2) {
                            vec![TransportKind::Vless]
                        } else {
                            vec![TransportKind::NativeUdp]
                        },
                    })
                    .collect(),
                allowed_paths: vec![forward.clone(), reverse.clone()],
                listeners,
                outgoing,
                limits: Limits::default(),
            },
            vless_listeners: if is_lax && use_vless {
                vec![VlessListener {
                    bind: SocketAddr::from((Ipv4Addr::UNSPECIFIED, vless.port())),
                    credential: uuid.clone(),
                    target,
                }]
            } else {
                vec![]
            },
            vless_outgoing: if *name == "sha" && use_vless {
                vec![VlessOutgoing {
                    peer: "lax".into(),
                    endpoint: vless,
                    credential: uuid.clone(),
                    target,
                    priority: 10,
                }]
            } else {
                vec![]
            },
            tun: if tun && !is_lax {
                Some(TunConfig {
                    name: "mwlab0".into(),
                    local_ipv4: Ipv4Addr::new(198, 18, 0, if *name == "nyc" { 1 } else { 2 }),
                    peer_ipv4: Ipv4Addr::new(198, 18, 0, if *name == "nyc" { 2 } else { 1 }),
                    send_path: if *name == "nyc" {
                        forward.clone()
                    } else {
                        reverse.clone()
                    },
                    receive_path: if *name == "nyc" {
                        reverse.clone()
                    } else {
                        forward.clone()
                    },
                    mtu: 1280,
                })
            } else {
                None
            },
        };
        config.validate()?;
        local::atomic_write(
            &directory.join(format!("{name}.json")),
            &serde_json::to_vec_pretty(&config)?,
            false,
        )?;
    }
    println!(
        "Created private nyc/lax/sha lab configurations; no controller identity was modified."
    );
    Ok(())
}
