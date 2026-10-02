//! Ignored isolated fixture. No production environment switch or protocol DTO.
use super::*;
use crate::managed_dataplane::{PacketPlan, TunOptions, tun::Tun};
use meshway_dataplane::{
    Identity, ListenerConfig, OutgoingCandidate, PeerConfig, PrivateKey, Runtime, TransportKind,
    packet_io::Device,
};
use meshway_protocol::local;
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Arc};
use tokio::{sync::watch, time::Instant as TokioInstant};

#[derive(Serialize, Deserialize)]
struct Fixture {
    runtime: LabConfig,
    payload: meshway_protocol::NodeMeshExecution,
    path: Vec<String>,
    prefix: String,
    role: String,
    underlay: Vec<Ipv4Addr>,
    lease_seconds: u64,
    authorized: bool,
    defaults: bool,
    locked: bool,
    direct_count: usize,
    internet_only: bool,
}

#[test]
#[ignore = "private isolated Docker exit plan generation"]
fn generate() {
    let directory = PathBuf::from(std::env::var("MESHWAY_EXIT_DIRECTORY").unwrap());
    let endpoints: Vec<std::net::SocketAddr> = std::env::var("MESHWAY_EXIT_ENDPOINTS")
        .unwrap()
        .split(',')
        .map(|v| v.parse().unwrap())
        .collect();
    let prefix = std::env::var("MESHWAY_EXIT_PREFIX").unwrap();
    let authorized = std::env::var("MESHWAY_EXIT_AUTHORIZED").unwrap() == "1";
    let lease_seconds = std::env::var("MESHWAY_EXIT_LEASE")
        .unwrap()
        .parse()
        .unwrap();
    let defaults = std::env::var("MESHWAY_EXIT_DEFAULTS").unwrap() == "1";
    let locked = std::env::var("MESHWAY_EXIT_LOCKED").unwrap() == "1";
    let direct_count = std::env::var("MESHWAY_EXIT_DIRECT_COUNT")
        .unwrap()
        .parse()
        .unwrap();
    let internet_only = std::env::var("MESHWAY_EXIT_INTERNET_ONLY").unwrap() == "1";
    assert!((2..=3).contains(&endpoints.len()));
    let (mut payloads, _) = crate::managed_dataplane::tests::fixtures(endpoints.len());
    let keys: Vec<_> = (0..endpoints.len())
        .map(|_| Identity::generate().unwrap())
        .collect();
    let ids: Vec<_> = payloads.iter().map(|p| p.node_id.clone()).collect();
    local::private_dir(&directory).unwrap();
    for (index, payload) in payloads.iter_mut().enumerate() {
        payload.network.ipv4_prefix = "198.19.90.0/24".into();
        payload.node.mesh_ipv4 = format!("198.19.90.{}", index + 1);
        for peer in &mut payload.peers {
            let n = usize::from_str_radix(&peer.id, 16).unwrap();
            peer.mesh_ipv4 = format!("198.19.90.{n}");
        }
        let path = if defaults {
            vec![
                ids[0].clone(),
                ids[if index == 0 { 1 } else { index }].clone(),
            ]
        } else {
            ids.clone()
        };
        let allowed_paths = if defaults && index == 0 {
            (1..ids.len())
                .flat_map(|i| {
                    [
                        vec![ids[0].clone(), ids[i].clone()],
                        vec![ids[i].clone(), ids[0].clone()],
                    ]
                })
                .collect()
        } else {
            vec![path.clone(), path.iter().rev().cloned().collect()]
        };
        let adjacent = |i: usize| {
            if defaults {
                i == 0 || index == 0
            } else {
                i.abs_diff(index) == 1
            }
        };
        let listeners = if (defaults && index == 0) || (!defaults && index + 1 < keys.len()) {
            vec![ListenerConfig {
                transport: TransportKind::NativeUdp,
                bind: format!("0.0.0.0:{}", endpoints[index].port())
                    .parse()
                    .unwrap(),
                id: None,
                allowed_peers: Some(if defaults {
                    ids[1..].to_vec()
                } else {
                    vec![ids[index + 1].clone()]
                }),
            }]
        } else {
            vec![]
        };
        let previous = if defaults { 0 } else { index.saturating_sub(1) };
        let runtime = LabConfig {
            node_id: ids[index].clone(),
            private_key: PrivateKey::from_hex(&keys[index].private_key_hex()).unwrap(),
            public_key: keys[index].public_key_hex(),
            trusted_peers: (0..keys.len())
                .filter(|i| *i != index)
                .map(|i| PeerConfig {
                    node_id: ids[i].clone(),
                    public_key: keys[i].public_key_hex(),
                    transports: if adjacent(i) {
                        vec![TransportKind::NativeUdp]
                    } else {
                        vec![]
                    },
                })
                .collect(),
            allowed_paths,
            listeners,
            outgoing: if index > 0 {
                vec![OutgoingCandidate {
                    peer: ids[previous].clone(),
                    transport: TransportKind::NativeUdp,
                    endpoint: endpoints[previous].to_string(),
                    priority: 0,
                    id: None,
                }]
            } else {
                vec![]
            },
            limits: Default::default(),
        };
        runtime.validate().unwrap();
        let underlay = if !defaults && endpoints.len() == 3 && index == 0 {
            // R's C-facing ephemeral carrier has the a-network source, while
            // its advertised listener for E is on b. Protect the actual peer.
            vec!["172.29.231.11".parse().unwrap()]
        } else {
            endpoints
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != index && adjacent(*i))
                .map(|(_, e)| match e.ip() {
                    std::net::IpAddr::V4(ip) => ip,
                    _ => panic!("IPv4 fixture"),
                })
                .collect()
        };
        let fixture = Fixture {
            runtime,
            payload: payload.clone(),
            path,
            prefix: prefix.clone(),
            role: if index == 0 {
                "client"
            } else if defaults || index + 1 == keys.len() {
                "exit"
            } else {
                "relay"
            }
            .into(),
            underlay,
            lease_seconds,
            authorized,
            defaults,
            locked,
            direct_count,
            internet_only,
        };
        local::atomic_write(
            &directory.join(format!("node-{index}.json")),
            &serde_json::to_vec(&fixture).unwrap(),
            false,
        )
        .unwrap();
    }
}

fn plan(f: &Fixture) -> ExitPlan {
    let local = f.payload.node.mesh_ipv4.parse().unwrap();
    let route = |path: Vec<String>, prefix| ExitRoute {
        return_path: path.iter().rev().cloned().collect(),
        path,
        prefix,
    };
    let routes = if f.role == "client" && f.prefix != "0.0.0.0/0" {
        vec![route(f.path.clone(), f.prefix.parse().unwrap())]
    } else {
        vec![]
    };
    let grants = if f.role == "exit" && f.authorized {
        vec![ExitGrant {
            authorization: ExitAuthorization {
                internet: f.internet_only,
                networks: if f.internet_only {
                    vec![]
                } else {
                    vec!["172.29.233.0/24".parse().unwrap()]
                },
            },
            source: "198.19.90.1".parse().unwrap(),
            path: f.path.clone(),
            return_path: f.path.iter().rev().cloned().collect(),
            prefixes: if f.defaults && f.runtime.node_id == format!("{:064x}", 3) {
                // Controller's route/authorization intersection: the public
                // S.20 rule always uses E, so F may not carry that destination.
                let host = u32::from(Ipv4Addr::new(172, 29, 233, 20));
                (1..=32)
                    .map(|length| {
                        let bit = 1u32 << (32 - length);
                        let network = (host & (u32::MAX << (32 - length))) ^ bit;
                        Ipv4Net::new(Ipv4Addr::from(network), length).unwrap()
                    })
                    .collect()
            } else {
                vec![if f.defaults {
                    "0.0.0.0/0".parse().unwrap()
                } else {
                    f.prefix.parse().unwrap()
                }]
            },
        }]
    } else {
        vec![]
    };
    let mut plan = ExitPlan::new(
        local,
        "198.19.90.0/24".parse().unwrap(),
        1000,
        routes,
        grants,
    );
    if f.role == "client" {
        plan = plan.with_direct(
            (0..f.direct_count)
                .map(|n| {
                    Ipv4Net::new(
                        Ipv4Addr::from(
                            u32::from(Ipv4Addr::new(10, 128, 0, 0)) + u32::try_from(n).unwrap(),
                        ),
                        32,
                    )
                    .unwrap()
                })
                .collect(),
        );
    }
    if f.role == "client" {
        let candidates: Vec<ExitRoute> = if f.defaults {
            f.runtime
                .allowed_paths
                .iter()
                .filter(|p| p.first() == Some(&f.runtime.node_id))
                .map(|p| route(p.clone(), "0.0.0.0/0".parse().unwrap()))
                .collect()
        } else {
            vec![route(f.path.clone(), "0.0.0.0/0".parse().unwrap())]
        };
        let mut profiles: Vec<_> = candidates
            .into_iter()
            .enumerate()
            .map(|(index, candidate)| RoutingProfile {
                id: if index == 0 { "e" } else { "f" }.into(),
                routes: plan.specific_routes.clone(),
                direct_prefixes: if index == 0 {
                    plan.direct_prefixes.clone()
                } else {
                    vec!["172.29.231.20/32".parse().unwrap()]
                },
                fallback: if f.defaults || f.prefix == "0.0.0.0/0" {
                    ProfileFallback::Exit(candidate)
                } else {
                    ProfileFallback::Unmanaged
                },
            })
            .collect();
        profiles.push(RoutingProfile {
            id: "mesh".into(),
            routes: plan.specific_routes.clone(),
            direct_prefixes: plan.direct_prefixes.clone(),
            fallback: ProfileFallback::Direct,
        });
        let automatic = if f.defaults {
            Some(AutomaticProfile {
                region_to_profile: BTreeMap::from([
                    ("US".into(), "e".into()),
                    ("JP".into(), "f".into()),
                ]),
                resolved_profile: Some("e".into()),
                fallback_profile: "mesh".into(),
            })
        } else {
            None
        };
        plan = plan
            .with_profiles(
                ProfileCatalog {
                    profiles,
                    default_profile: "e".into(),
                    locked: f.locked,
                    automatic,
                },
                None,
            )
            .unwrap();
    }
    plan.validate(&f.runtime).unwrap();
    plan
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires isolated NET_ADMIN TUN nft conntrack namespace"]
async fn serve() {
    let directory = PathBuf::from("/run/meshway-exit");
    let f: Fixture =
        serde_json::from_str(&local::read_private(&directory.join("config.json")).unwrap())
            .unwrap();
    let mut plan = Arc::new(plan(&f));
    let packet = PacketPlan::sink(&f.payload, 1000).unwrap();
    let tun = Tun::open(
        &TunOptions {
            name: "mwexit0".into(),
            mtu: 1000,
        },
        &packet,
        "http://127.0.0.1:8420",
        false,
    )
    .await
    .unwrap();
    let started = std::time::Instant::now();
    let mut routes = if f.role == "client" {
        Some(
            linux::ClientRoutes::install(&plan, &tun.name, &f.underlay)
                .await
                .unwrap(),
        )
    } else {
        None
    };
    std::fs::write(directory.join("install.json"),serde_json::to_vec(&serde_json::json!({"milliseconds":started.elapsed().as_secs_f64()*1000.,"direct_prefixes":plan.direct_prefixes.len()})).unwrap()).unwrap();
    let firewall = if f.role == "exit" && f.authorized {
        Some(
            linux::ExitFirewall::install(&plan, &tun.name, "eth1")
                .await
                .unwrap(),
        )
    } else {
        None
    };
    let until = TokioInstant::now() + Duration::from_secs(f.lease_seconds);
    let mut generation = 0;
    let runtime = Arc::new(Runtime::start(f.runtime.clone()).await.unwrap());
    let watchdog = tokio::spawn({
        let runtime = runtime.clone();
        async move {
            tokio::time::sleep_until(until).await;
            runtime.shutdown().await;
        }
    });
    loop {
        generation += 1;
        let (send, receive) = watch::channel(Some(until));
        let (action, retirement) = watch::channel(linux::Retirement::Withdraw);
        let mut successor = None;
        let ready=serde_json::to_vec(&serde_json::json!({"generation":generation,"revision":f.payload.revision,"selected_profile":plan.selected_profile,"effective_profile":plan.effective_profile})).unwrap();
        let io = linux::run(
            tun.device.clone(),
            runtime.clone(),
            plan.clone(),
            receive,
            firewall.as_ref(),
            retirement,
            || {
                local::atomic_write(
                    &directory.join("ready"),
                    &ready,
                    directory.join("ready").exists(),
                )
                .unwrap();
            },
        );
        tokio::pin!(io);
        let mut tick = tokio::time::interval(Duration::from_millis(100));
        loop {
            tokio::select! {
                result=&mut io => {result.unwrap();break;},
                _=tick.tick() => {
                    if directory.join("replace-request").exists() && successor.is_none() {
                        std::fs::remove_file(directory.join("replace-request")).unwrap();
                         let mut catalog=plan.profiles.clone().unwrap();
                         for profile in &mut catalog.profiles {profile.direct_prefixes=(0..f.direct_count).map(|n|Ipv4Net::new(Ipv4Addr::from(u32::from(Ipv4Addr::new(10,129,0,0))+u32::try_from(n).unwrap()),32).unwrap()).collect();}
                         let next=ExitPlan::new(plan.local,plan.mesh,plan.mtu,vec![],vec![]).with_profiles(catalog,plan.selected_profile.as_deref()).unwrap();
                         next.validate(&f.runtime).unwrap();successor=Some(next);action.send_replace(linux::Retirement::ReplaceAdmission);send.send_replace(None);
                    }
                    if directory.join("withdraw").exists() || directory.join("restore-local").exists() {send.send_replace(None);}
                    if f.role=="client" && directory.join("selection-request").exists() && successor.is_none() {
                        let request=std::fs::read_to_string(directory.join("selection-request")).unwrap();
                        std::fs::remove_file(directory.join("selection-request")).unwrap();
                         match plan.select_profile(request.trim()) {
                            Ok(next) => {next.validate(&f.runtime).unwrap();successor=Some(next);action.send_replace(linux::Retirement::ReplaceAdmission);send.send_replace(None);},
                            Err(error) => {std::fs::write(directory.join("selection-result"),format!("rejected: {error}")).unwrap();},
                         }
                     }
                     if directory.join("auto-resolution").exists() && successor.is_none() {
                         let region=std::fs::read_to_string(directory.join("auto-resolution")).unwrap();
                         std::fs::remove_file(directory.join("auto-resolution")).unwrap();
                         let resolved=plan.profiles.as_ref().unwrap().automatic.as_ref().unwrap().region_to_profile.get(region.trim());
                         let next=plan.resolve_automatic(resolved.map(String::as_str)).unwrap();
                         next.validate(&f.runtime).unwrap();successor=Some(next);action.send_replace(linux::Retirement::ReplaceAdmission);send.send_replace(None);
                     }
                    if directory.join("inject-forged").exists() {
                        std::fs::remove_file(directory.join("inject-forged")).unwrap();
                        let mut forged=crate::managed_dataplane::tests::packet("198.19.90.99",&std::env::var("MESHWAY_EXIT_TARGET").unwrap(),17);
                        forged[20..22].copy_from_slice(&40000u16.to_be_bytes());forged[22..24].copy_from_slice(&9001u16.to_be_bytes());forged[24..26].copy_from_slice(&8u16.to_be_bytes());
                        runtime.send_packet(f.path.clone(),forged).await.unwrap();
                        std::fs::write(directory.join("forged-sent"),b"sent").unwrap();
                    }
                    if directory.join("inject-unsolicited").exists() {
                        std::fs::remove_file(directory.join("inject-unsolicited")).unwrap();
                        let mut forged=crate::managed_dataplane::tests::packet(&std::env::var("MESHWAY_EXIT_TARGET").unwrap(),"198.19.90.1",17);
                        forged[20..22].copy_from_slice(&9001u16.to_be_bytes());forged[22..24].copy_from_slice(&9001u16.to_be_bytes());forged[24..26].copy_from_slice(&8u16.to_be_bytes());
                        runtime.send_packet(f.path.iter().rev().cloned().collect(),forged).await.unwrap();
                        std::fs::write(directory.join("unsolicited-sent"),b"sent").unwrap();
                    }
                    if directory.join("inject-wrong-exit").exists() {
                        std::fs::remove_file(directory.join("inject-wrong-exit")).unwrap();
                        let path=f.runtime.allowed_paths.iter().find(|p|p.first()==Some(&f.runtime.node_id) && p.last()==Some(&format!("{:064x}",3))).unwrap();
                        let mut forged=crate::managed_dataplane::tests::packet("198.19.90.1",&std::env::var("MESHWAY_EXIT_TARGET").unwrap(),17);
                        forged[20..22].copy_from_slice(&40000u16.to_be_bytes());forged[22..24].copy_from_slice(&9001u16.to_be_bytes());forged[24..26].copy_from_slice(&8u16.to_be_bytes());
                        runtime.send_packet(path.clone(),forged).await.unwrap();
                        std::fs::write(directory.join("wrong-exit-sent"),b"sent").unwrap();
                    }
                },
            }
        }
        if let Some(next) = successor {
            {
                let started = std::time::Instant::now();
                routes
                    .as_mut()
                    .unwrap()
                    .replace(&next, &tun.name)
                    .await
                    .unwrap();
                std::fs::write(
                    directory.join("replace.json"),
                    serde_json::to_vec(
                        &serde_json::json!({"milliseconds":started.elapsed().as_secs_f64()*1000.}),
                    )
                    .unwrap(),
                )
                .unwrap();
            }
            tun.device.drain().unwrap();
            plan = Arc::new(next);
            std::fs::write(directory.join("selection-result"), b"accepted").unwrap();
            continue;
        }
        if directory.join("restore-local").exists()
            && let Some(routes) = routes.take()
        {
            let started = std::time::Instant::now();
            routes.restore_local().await.unwrap();
            std::fs::write(
                directory.join("remove.json"),
                serde_json::to_vec(
                    &serde_json::json!({"milliseconds":started.elapsed().as_secs_f64()*1000.}),
                )
                .unwrap(),
            )
            .unwrap();
        }
        runtime.shutdown().await;
        watchdog.abort();
        std::fs::write(directory.join("stopped"), b"withdrawn").unwrap();
        std::future::pending::<()>().await;
    }
}
