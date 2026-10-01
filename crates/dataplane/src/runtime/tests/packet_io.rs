use super::*;
use crate::packet_io::{self, ChannelDevice, Device, Ipv4PacketPlan, PacketBatch};
use crate::{MAX_BATCH_FRAMES, MAX_PACKET_PAYLOAD};
use std::{collections::BTreeMap, future::Future, net::Ipv4Addr};
use tokio::{sync::watch, time};

struct ObservedDevice {
    channel: Arc<ChannelDevice>,
    writes: tokio::sync::Semaphore,
}

impl Device for ObservedDevice {
    async fn recv_batch(&self) -> std::io::Result<PacketBatch> {
        self.channel.recv_batch().await
    }
    async fn send_batch(&self, packets: PacketBatch) -> std::io::Result<()> {
        self.writes.add_permits(1);
        self.channel.send_batch(packets).await
    }
    fn drain(&self) -> std::io::Result<()> {
        self.channel.drain()
    }
}

fn batch(markers: &[u8]) -> PacketBatch {
    PacketBatch::new(markers.iter().map(|marker| vec![*marker]).collect()).unwrap()
}

#[tokio::test]
async fn bounded_callback_queues_order_drain_and_cancelled_writes() {
    assert!(packet_io::channel(0).is_err());
    assert!(packet_io::channel(257).is_err());
    let (device, endpoint) = packet_io::channel(1).unwrap();
    endpoint.try_feed(batch(&[1, 2, 3])).unwrap();
    let rejected = endpoint.try_feed(batch(&[4])).unwrap_err();
    assert!(matches!(rejected, mpsc::error::TrySendError::Full(_)));
    assert_eq!(rejected.into_inner().into_packets(), vec![vec![4]]);
    assert_eq!(
        device.recv_batch().await.unwrap().into_packets(),
        vec![vec![1], vec![2], vec![3]]
    );
    endpoint.try_feed(batch(&[5])).unwrap();
    device.drain().unwrap();
    endpoint.try_feed(batch(&[6])).unwrap();
    assert_eq!(
        device.recv_batch().await.unwrap().into_packets(),
        vec![vec![6]]
    );

    device.send_batch(batch(&[7, 8])).await.unwrap();
    {
        let blocked = device.send_batch(batch(&[9]));
        tokio::pin!(blocked);
        assert!(matches!(
            std::future::poll_fn(|cx| Poll::Ready(blocked.as_mut().poll(cx))).await,
            Poll::Pending
        ));
        // Dropping a blocked write cannot enqueue later after the callback drains.
    }
    endpoint.drain().unwrap();
    device.send_batch(batch(&[10])).await.unwrap();
    assert_eq!(
        endpoint.recv_batch().await.unwrap().into_packets(),
        vec![vec![10]]
    );
    drop(endpoint);
    assert!(device.recv_batch().await.is_err());
    assert!(device.send_batch(batch(&[11])).await.is_err());
}

#[test]
fn packet_batch_bounds_are_checked_before_callback_admission() {
    assert!(PacketBatch::new(vec![]).is_err());
    assert!(PacketBatch::new(vec![vec![]]).is_err());
    assert!(PacketBatch::new(vec![vec![0; MAX_PACKET_PAYLOAD + 1]]).is_err());
    assert!(PacketBatch::new(vec![vec![1]; MAX_BATCH_FRAMES + 1]).is_err());
    assert!(PacketBatch::new(vec![vec![0; MAX_PACKET_PAYLOAD]; 9]).is_err());
    assert!(PacketBatch::new(vec![vec![0; MAX_PACKET_PAYLOAD]; 8]).is_ok());
}

fn ipv4(marker: u8) -> Vec<u8> {
    let mut bytes = vec![0; 28];
    bytes[0] = 0x45;
    bytes[2..4].copy_from_slice(&28_u16.to_be_bytes());
    bytes[6] = 0x40;
    bytes[8] = 64;
    bytes[9] = marker;
    bytes[12..16].copy_from_slice(&[100, 64, 0, 1]);
    bytes[16..20].copy_from_slice(&[100, 64, 0, 3]);
    let mut sum: u32 = bytes[..20]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|word| u32::from(u16::from_be_bytes(*word)))
        .sum();
    while sum > 0xffff {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    bytes[10..12].copy_from_slice(&(!(sum as u16)).to_be_bytes());
    bytes
}

#[tokio::test]
async fn callback_batch_drives_real_encrypted_multihop_pump_in_order() {
    let fixture = Fixture::native(TransportKind::NativeTcp).await;
    let source_ip = Ipv4Addr::new(100, 64, 0, 1);
    let target_ip = Ipv4Addr::new(100, 64, 0, 3);
    let source_plan = Ipv4PacketPlan::new(
        source_ip,
        1000,
        BTreeMap::from([(target_ip, path())]),
        BTreeMap::new(),
    );
    let target_plan = Ipv4PacketPlan::new(
        target_ip,
        1000,
        BTreeMap::new(),
        BTreeMap::from([(path(), source_ip)]),
    );
    let (source, source_callback) = packet_io::channel(2).unwrap();
    let (target, target_callback) = packet_io::channel(16).unwrap();
    let (lease, deadline) = watch::channel(Some(Instant::now() + Duration::from_secs(60)));
    let sender = tokio::spawn(packet_io::run(
        source.clone(),
        Arc::new(fixture.nyc.clone()),
        source_plan,
        deadline.clone(),
    ));
    let receiver = tokio::spawn(packet_io::run(
        target.clone(),
        Arc::new(fixture.sha.clone()),
        target_plan,
        deadline,
    ));
    let expected: Vec<_> = (1..=16).map(ipv4).collect();
    // A batch-valid but over-MTU packet and IPv6/malformed input must not reach
    // encryption or the destination device.
    source_callback
        .try_feed(PacketBatch::new(vec![vec![0; 1001], vec![0x60; 28]]).unwrap())
        .unwrap();
    source_callback
        .try_feed(PacketBatch::new(expected.clone()).unwrap())
        .unwrap();
    for packet in expected {
        let delivered = time::timeout(Duration::from_secs(3), target_callback.recv_batch())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(delivered.into_packets(), vec![packet]);
    }
    lease.send_replace(None);
    time::timeout(Duration::from_secs(1), sender)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    time::timeout(Duration::from_secs(1), receiver)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    source.drain().unwrap();
    target.drain().unwrap();
    source_callback.drain().unwrap();
    target_callback.drain().unwrap();
    fixture.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn lease_expiry_and_closed_watch_cancel_blocked_receive() {
    for closed in [false, true] {
        let runtime = Arc::new(Runtime::start(configs().remove(0)).await.unwrap());
        let (device, _endpoint) = packet_io::channel(1).unwrap();
        let plan = Ipv4PacketPlan::new(
            Ipv4Addr::new(100, 64, 0, 1),
            1000,
            BTreeMap::new(),
            BTreeMap::new(),
        );
        let (lease, deadline) = watch::channel(Some(Instant::now() + Duration::from_secs(5)));
        let pump = tokio::spawn(packet_io::run(
            device.clone(),
            runtime.clone(),
            plan,
            deadline,
        ));
        tokio::task::yield_now().await;
        if closed {
            drop(lease);
        } else {
            time::advance(Duration::from_secs(5)).await;
        }
        pump.await.unwrap().unwrap();
        device.drain().unwrap();
        runtime.shutdown().await;
    }
}

#[tokio::test]
async fn withdrawal_cancels_a_full_callback_write_without_late_delivery() {
    let fixture = Fixture::native(TransportKind::NativeTcp).await;
    let (channel, endpoint) = packet_io::channel(1).unwrap();
    let device = Arc::new(ObservedDevice {
        channel,
        writes: tokio::sync::Semaphore::new(0),
    });
    let plan = Ipv4PacketPlan::new(
        Ipv4Addr::new(100, 64, 0, 3),
        1000,
        BTreeMap::new(),
        BTreeMap::from([(path(), Ipv4Addr::new(100, 64, 0, 1))]),
    );
    let (lease, deadline) = watch::channel(Some(Instant::now() + Duration::from_secs(60)));
    let pump = tokio::spawn(packet_io::run(
        device.clone(),
        Arc::new(fixture.sha.clone()),
        plan,
        deadline,
    ));
    fixture
        .nyc
        .send_packet_batch(path(), vec![ipv4(1), ipv4(2)])
        .await
        .unwrap();
    // The first write fills the sole callback queue slot. The second attempt
    // blocks, so withdrawal must interrupt the write rather than await its consumer.
    time::timeout(Duration::from_secs(3), device.writes.acquire_many(2))
        .await
        .unwrap()
        .unwrap()
        .forget();
    lease.send_replace(None);
    time::timeout(Duration::from_secs(1), pump)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(
        endpoint.recv_batch().await.unwrap().into_packets(),
        vec![ipv4(1)]
    );
    device.channel.send_batch(batch(&[3])).await.unwrap();
    assert_eq!(
        endpoint.recv_batch().await.unwrap().into_packets(),
        vec![vec![3]]
    );
    device.drain().unwrap();
    fixture.shutdown().await;
}

#[tokio::test]
async fn activation_observation_requires_unique_receiver_and_live_generation() {
    for (receiver_taken, live) in [(true, true), (false, false), (false, true)] {
        let runtime = Arc::new(Runtime::start(configs().remove(0)).await.unwrap());
        let _other_receiver = receiver_taken.then(|| runtime.take_packet_receiver().unwrap());
        let (device, _endpoint) = packet_io::channel(1).unwrap();
        let plan = Ipv4PacketPlan::new(
            Ipv4Addr::new(100, 64, 0, 1),
            1000,
            BTreeMap::new(),
            BTreeMap::new(),
        );
        let (lease, deadline) =
            watch::channel(live.then(|| Instant::now() + Duration::from_secs(60)));
        let observed = std::sync::atomic::AtomicBool::new(false);
        let result =
            packet_io::run_with_activation(device, runtime.clone(), plan, deadline, || {
                observed.store(true, std::sync::atomic::Ordering::SeqCst);
                lease.send_replace(None);
            })
            .await;
        assert_eq!(result.is_err(), receiver_taken);
        assert_eq!(
            observed.load(std::sync::atomic::Ordering::SeqCst),
            !receiver_taken && live
        );
        runtime.shutdown().await;
    }
}
