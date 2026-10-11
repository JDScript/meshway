//! End to end against the real Server: three Nodes in their sync loops pin
//! the rotated netmap signing key within one hold and keep renewing after the
//! rotation window ends.
use super::support::Server;
use meshway_control::{Control, sync_loop};
use meshway_protocol::PinnedServerKeyStatus;
use meshway_test_server::client::ListNodesPath;
use std::{sync::Arc, time::Duration};
use tokio::sync::{Mutex, watch};
use tokio_util::sync::CancellationToken;

struct Running {
    control: Arc<Mutex<Control>>,
    netmaps: watch::Receiver<Option<meshway_engine::Netmap>>,
    task: tokio::task::JoinHandle<Result<(), meshway_control::Error>>,
}

fn run(control: Control, stop: &CancellationToken) -> Running {
    let control = Arc::new(Mutex::new(control));
    let (sender, netmaps) = watch::channel(None);
    let task = tokio::spawn({
        let (control, stop) = (Arc::clone(&control), stop.clone());
        async move {
            let mut facts = meshway_test_server::Facts::default();
            sync_loop(&control, &mut facts, &sender, &stop).await
        }
    });
    Running {
        control,
        netmaps,
        task,
    }
}

async fn pinned_generation(node: &Running) -> u64 {
    node.control
        .lock()
        .await
        .status()
        .binding
        .unwrap()
        .server_signing_key
        .key_generation
        .get()
}

#[tokio::test]
async fn three_nodes_pin_the_rotated_key_within_one_hold_and_renew_after_the_window() {
    let server = Server::start().await;
    let owner = server.owner().await;
    let stop = CancellationToken::new();
    let mut nodes = Vec::new();
    for name in ["rotate-a", "rotate-b", "rotate-c"] {
        let control = server
            .control(&owner, &server.node_origin, name, Default::default())
            .await;
        nodes.push(run(control, &stop));
    }
    for node in &mut nodes {
        node.netmaps.wait_for(Option::is_some).await.unwrap();
        assert_eq!(pinned_generation(node).await, 1);
    }

    let window = Duration::from_secs(60);
    let started = tokio::time::Instant::now();
    let rotated = server.rotate_netmap_key(window).await;
    assert_eq!(rotated.next.key_generation.get(), 2);
    // The fixture hold is 1 s (±10%) and an immediate unchanged answer is
    // spaced by 1 s: every Node pins the successor within a few seconds.
    for node in &nodes {
        tokio::time::timeout(Duration::from_secs(5), async {
            while pinned_generation(node).await != 2 {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("pinned within one hold");
    }
    let pinned_after = started.elapsed();
    // The inventory reflects what the next request reports.
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let inventory = server
                .client
                .list_nodes(
                    &ListNodesPath {
                        network_id: owner.network,
                    },
                    owner.token.expose(),
                )
                .await
                .unwrap();
            if inventory.items.len() == 3
                && inventory.items.iter().all(|item| {
                    item.server_key.as_ref().is_some_and(|key| {
                        key.key_generation.get() == 2
                            && key.status == PinnedServerKeyStatus::Current
                    })
                })
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("inventory shows the current key (pinned after {pinned_after:?})"));

    // After the window the successor alone signs; the loops keep renewing.
    let ends = u64::try_from(chrono::Utc::now().timestamp_millis()).unwrap();
    let remaining = rotated.window_ends_at_ms.saturating_sub(ends);
    tokio::time::sleep(Duration::from_millis(remaining)).await;
    for node in &mut nodes {
        let renewed = tokio::time::timeout(
            Duration::from_secs(5),
            node.netmaps.wait_for(|netmap| {
                netmap
                    .as_ref()
                    .is_some_and(|netmap| netmap.lease.issued_at_ms > rotated.window_ends_at_ms)
            }),
        )
        .await
        .expect("renewed after the window")
        .unwrap()
        .clone()
        .unwrap();
        assert!(renewed.lease.expires_at_ms > rotated.window_ends_at_ms);
        assert_eq!(pinned_generation(node).await, 2);
        assert!(!node.task.is_finished(), "the sync loop keeps running");
    }
    stop.cancel();
    for node in nodes {
        node.task.await.unwrap().unwrap();
        Arc::try_unwrap(node.control)
            .ok()
            .unwrap()
            .into_inner()
            .shutdown()
            .await
            .unwrap();
    }
    server.close().await;
}
