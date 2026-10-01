//! Explicitly ignored, isolated Docker performance fixture. Never enabled in a
//! production binary; reuse generated contracts and the real managed Manager.
use super::TunOptions;
use crate::managed_connections::Manager;
use meshway_dataplane::PrivateKey;
use meshway_protocol::{ConnectionRuntimeReason, NodeDataplane, local};
use serde::{Deserialize, Serialize};
use std::{net::SocketAddr, path::PathBuf, time::Duration};
use tokio::time::Instant;

// Ignored fixture observability only. A Weak never extends a running generation
// or retains its keys; production builds have neither registration nor sampler.
static RUNTIME: std::sync::LazyLock<std::sync::Mutex<std::sync::Weak<meshway_dataplane::Runtime>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(std::sync::Weak::new()));

pub(super) fn observe_runtime(runtime: &std::sync::Arc<meshway_dataplane::Runtime>) {
    *RUNTIME.lock().unwrap() = std::sync::Arc::downgrade(runtime);
}

#[derive(Serialize, Deserialize)]
struct Fixture {
    payload: NodeDataplane,
    private_key: PrivateKey,
}

#[test]
#[ignore = "requires isolated native Docker namespace; generated keys are private"]
fn generate() {
    let directory = PathBuf::from(std::env::var("MESHWAY_THROUGHPUT_DIRECTORY").unwrap());
    let endpoints: Vec<SocketAddr> = std::env::var("MESHWAY_THROUGHPUT_ENDPOINTS")
        .unwrap()
        .split(',')
        .map(|value| value.parse().unwrap())
        .collect();
    assert!((2..=3).contains(&endpoints.len()));
    local::private_dir(&directory).unwrap();
    let (payloads, keys) = super::tests::fixtures(endpoints.len());
    for (index, (mut payload, private_key)) in payloads.into_iter().zip(keys).enumerate() {
        payload.configuration.network.ipv4_prefix = "198.18.1.0/24".into();
        payload.configuration.node.mesh_ipv4 = format!("198.18.1.{}", index + 1);
        for node in &mut payload.configuration.peers {
            let ordinal = usize::from_str_radix(&node.id, 16).unwrap();
            node.mesh_ipv4 = format!("198.18.1.{ordinal}");
        }
        for listener in &mut payload.connections.listeners {
            listener.bind = format!("0.0.0.0:{}", endpoints[index].port());
            for endpoint in &mut listener.advertised_endpoints {
                endpoint.host = endpoints[index].ip().to_string();
                endpoint.port = endpoints[index].port();
            }
        }
        for candidate in &mut payload.connections.outgoing {
            candidate.endpoint.host = endpoints[index - 1].ip().to_string();
            candidate.endpoint.port = endpoints[index - 1].port();
        }
        let destination = directory.join(format!("node-{index}.json"));
        assert!(!destination.exists());
        local::atomic_write(
            &destination,
            &serde_json::to_vec(&Fixture {
                payload,
                private_key,
            })
            .unwrap(),
            false,
        )
        .unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires isolated NET_ADMIN/TUN namespace; 300-second bounded managed lease"]
async fn serve() {
    let file = PathBuf::from(std::env::var("MESHWAY_THROUGHPUT_CONFIG").unwrap());
    let status = PathBuf::from(std::env::var("MESHWAY_THROUGHPUT_STATUS").unwrap());
    let fixture: Fixture = serde_json::from_str(&local::read_private(&file).unwrap()).unwrap();
    let snapshot = super::tests::snapshot(fixture.payload);
    let mut manager = Manager::new_dataplane(
        None,
        Duration::from_secs(300),
        TunOptions {
            name: "mwlab0".into(),
            mtu: 1000,
        },
        "http://127.0.0.1:8420".into(),
        false,
    );
    // Real coherent plan compilation, authorized listener/candidate binding,
    // route sink, lease watchdog and managed packet task. No HTTP enrollment.
    manager
        .reconcile_dataplane(snapshot, fixture.private_key, Instant::now())
        .await
        .unwrap();
    let mut timer = tokio::time::interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            _ = timer.tick() => {
                let observation = manager.observation();
                local::atomic_write(&status, &serde_json::to_vec(&observation).unwrap(), status.try_exists().unwrap()).unwrap();
                if let Some(runtime) = RUNTIME.lock().unwrap().upgrade() {
                    let statistics = status.with_file_name("runtime-statistics.json");
                    local::atomic_write(&statistics, &serde_json::to_vec(&runtime.statistics()).unwrap(), statistics.try_exists().unwrap()).unwrap();
                }
            }
        }
    }
    manager.withdraw(ConnectionRuntimeReason::None).await;
}
