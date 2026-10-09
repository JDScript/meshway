//! Real worker/Host installation with one deliberately slow nft invocation.
//! The wrapper exists only inside the explicitly opted-in disposable container.
use super::*;
use crate::runtime::network::{Handle, Report};
use std::{os::unix::fs::PermissionsExt, path::PathBuf};

struct SlowNft {
    original: PathBuf,
    armed: PathBuf,
    entered: PathBuf,
}
impl SlowNft {
    fn install() -> Self {
        assert_eq!(std::env::var("MESHWAY_X4_SLOW_NFT").as_deref(), Ok("1"));
        let original = PathBuf::from("/usr/sbin/nft.x4-original");
        assert!(!original.exists());
        let armed = PathBuf::from("/root/meshway-tmp/nft-armed");
        let entered = PathBuf::from("/root/meshway-tmp/nft-entered");
        std::fs::rename("/usr/sbin/nft", &original).unwrap();
        let wrapper = Self {
            original,
            armed,
            entered,
        };
        std::fs::write("/usr/sbin/nft", b"#!/bin/sh\nif [ -f /root/meshway-tmp/nft-armed ]; then\n rm /root/meshway-tmp/nft-armed\n touch /root/meshway-tmp/nft-entered\n sleep 1.5\nfi\nexec /usr/sbin/nft.x4-original \"$@\"\n").unwrap();
        std::fs::set_permissions("/usr/sbin/nft", std::fs::Permissions::from_mode(0o755)).unwrap();
        wrapper
    }
    async fn entered(&self) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while !self.entered.exists() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }
}
impl Drop for SlowNft {
    fn drop(&mut self) {
        std::fs::rename(&self.original, "/usr/sbin/nft").unwrap();
    }
}

async fn state(f: &Fixture) -> meshway_protocol::NodeStateView {
    f.client
        .get_node_state(
            &meshway_control_client::GetNodeStatePath {
                network_id: f.network,
                node_id: f.nodes()[2],
            },
            f.token.expose(),
        )
        .await
        .unwrap()
        .nodes
        .remove(&f.nodes()[2])
        .unwrap()
}

async fn settled(handle: &mut Handle, f: &mut Fixture) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Ok(sample) = handle.sample().await
                && sample.report.current(&f.controls[2].status())
                && sample.report.completion == Some(Ok(()))
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    handle.finish(&mut f.controls[2]).await.unwrap();
}

#[tokio::test]
#[ignore = "requires disposable Linux Host and explicit MESHWAY_X4_SLOW_NFT=1"]
async fn busy_installation_skips_state_without_withdrawing_exit_then_stopped_withdraws() {
    isolated();
    let nft = SlowNft::install();
    let mut f = Fixture::open().await;
    let settings =
        crate::settings::Settings::new(f.directory.path().join("daemon"), &f.origin, Some("exit"))
            .unwrap();
    let (reports, _) = tokio::sync::watch::channel(Report::stopped());
    let mut handle = Handle::open(
        f.host_directory(),
        &f.origin,
        reports,
        Fixture::options(),
        meshway_dns::SystemDnsSettings::default(),
    )
    .await
    .unwrap();
    handle.publish(&mut f.controls[2]).await.unwrap();
    settled(&mut handle, &mut f).await;
    assert!(
        handle
            .report_state(&mut f.controls[2], &settings, true)
            .await
            .unwrap()
    );
    let demand = f.demand().await;
    handle.publish(&mut f.controls[2]).await.unwrap();
    settled(&mut handle, &mut f).await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(
        handle
            .report_state(&mut f.controls[2], &settings, true)
            .await
            .unwrap()
    );
    f.refresh().await;
    handle.publish(&mut f.controls[2]).await.unwrap();
    settled(&mut handle, &mut f).await;
    let mut mesh = Mesh::new(&f, demand);
    // Keep the admitted packet and its original permit, without fabricating a
    // Host projection ACK. This regression checks the authorization/circuit
    // boundary; the separate driver test checks actual NAT and return traffic.
    let packet_before = mesh.forward(packet(&f, 42000));
    let references: Vec<_> = f
        .controls
        .iter()
        .map(|c| c.status().cached.unwrap())
        .collect();
    let before = state(&f).await;
    let capability = handle.report.borrow().exit_capability.clone().unwrap();
    tokio::time::sleep(Duration::from_secs(1)).await;
    std::fs::write(&nft.armed, b"once").unwrap();
    // Reoffer the current installation, as with a routine lease refresh. The
    // actual worker executes Host/probe/nft before servicing Command::Sample.
    handle.target.send_modify(|_| {});
    nft.entered().await;
    let started = std::time::Instant::now();
    let submitted = handle
        .report_state(&mut f.controls[2], &settings, true)
        .await
        .unwrap();
    let elapsed = started.elapsed();
    let after = state(&f).await;
    f.refresh().await;
    let refreshed: Vec<_> = f
        .controls
        .iter()
        .map(|c| c.status().cached.unwrap())
        .collect();
    eprintln!(
        "X4B_SAMPLE elapsed={elapsed:?} submitted={submitted} sequence_before={:?} sequence_after={:?} publications_before={references:?} publications_after={refreshed:?}",
        before.report_sequence, after.report_sequence
    );
    assert!(elapsed >= Duration::from_secs(1));
    assert!(
        !submitted,
        "busy worker must not submit a fabricated stopped State"
    );
    assert_eq!(before.report_sequence, after.report_sequence);
    assert_eq!(before.observed_at, after.observed_at);
    assert_eq!(before.report_expires_at, after.report_expires_at);
    assert_eq!(before.report, after.report);
    assert_eq!(
        references, refreshed,
        "missing sample must not recompile authorization"
    );
    mesh.reconcile(&f);
    for node in [&mesh.a, &mesh.r, &mesh.e] {
        assert!(node.circuit_info(mesh.circuit).unwrap().is_some());
    }
    assert_eq!(mesh.a.retained_flows(), 1);
    assert_eq!(mesh.e.retained_flows(), 1);
    let packet_after = mesh.forward(packet(&f, 42000));
    assert_eq!(
        packet_before.exit_scope().unwrap().deadline(),
        packet_after.exit_scope().unwrap().deadline()
    );
    settled(&mut handle, &mut f).await;
    assert!(
        handle
            .report
            .borrow()
            .exit_capability
            .as_ref()
            .unwrap()
            .observed_at
            >= capability.observed_at
    );
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(
        handle
            .report_state(&mut f.controls[2], &settings, true)
            .await
            .unwrap()
    );
    assert_ne!(state(&f).await.report_sequence, before.report_sequence);

    // A successful sample after actual role teardown is absence, not a timeout.
    handle.stop(false).await.unwrap();
    assert!(
        handle
            .sample()
            .await
            .unwrap()
            .report
            .exit_capability
            .is_none()
    );
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(
        handle
            .report_state(&mut f.controls[2], &settings, true)
            .await
            .unwrap()
    );
    f.refresh().await;
    let retired: Vec<_> = f
        .controls
        .iter()
        .map(|c| c.status().cached.unwrap())
        .collect();
    assert_ne!(references, retired);
    mesh.reconcile(&f);
    for node in [&mesh.a, &mesh.r, &mesh.e] {
        assert!(node.circuit_info(mesh.circuit).unwrap().is_none());
    }
    eprintln!(
        "X4B_STOPPED sequence={:?} publications={retired:?}",
        state(&f).await.report_sequence
    );
    drop((packet_before, packet_after));
    handle.shutdown(false).await.unwrap();
    f.close().await;
}
