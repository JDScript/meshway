//! A host packet device must not bypass the original pending packet lifetime.
use super::{
    netmap_flow::{deliver, first, icmp, send},
    support::netmap::NetmapMesh,
};
use meshway_dataplane::{carrier, secure::Received};
use meshway_tun::{Io, PacketBatch, PacketDevice};
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Default)]
struct PrefixDevice {
    calls: AtomicUsize,
    committed: Mutex<Vec<Vec<u8>>>,
    prefix: tokio::sync::Notify,
}
impl PacketDevice for PrefixDevice {
    fn mtu(&self) -> usize {
        1500
    }
    fn recv_batch<'a>(&'a self, _: &'a mut PacketBatch) -> Io<'a, ()> {
        Box::pin(std::future::pending())
    }
    fn send_batch<'a>(&'a self, packets: &'a [&'a [u8]]) -> Io<'a, usize> {
        Box::pin(async move {
            if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
                self.committed.lock().unwrap().push(packets[0].to_vec());
                self.prefix.notify_one();
                Ok(1)
            } else {
                std::future::pending().await
            }
        })
    }
}

#[tokio::test]
async fn device_prefix_is_committed_once_and_withdrawal_cancels_its_pending_suffix() {
    let mut mesh = NetmapMesh::start().await;
    let packet = icmp(&mesh, false, 0);
    let circuit = first(&mut mesh, packet).await;
    let mut pending = Vec::new();
    for sequence in 1..=2 {
        let packet = icmp(&mesh, false, sequence);
        let frame = send(&mut mesh, packet);
        let Received::Packet(packet) = deliver(&mut mesh, frame).unwrap() else {
            panic!("packet delivery");
        };
        pending.push(packet);
    }
    let device = PrefixDevice::default();
    let write = carrier::deliver_batch(&device, pending);
    tokio::pin!(write);
    tokio::select! {
        _ = device.prefix.notified() => {},
        result = &mut write => panic!("suffix must still be pending: {result:?}"),
    }
    mesh.b.close(circuit);
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(1), write)
            .await
            .unwrap()
            .is_err()
    );
    assert_eq!(device.committed.lock().unwrap().len(), 1);
    mesh.close().await;
}
