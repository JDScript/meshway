use super::*;
use std::os::unix::net::UnixDatagram;

#[tokio::test]
async fn framing_cancellation_and_committed_prefix() {
    let (device, peer) = UnixDatagram::pair().unwrap();
    peer.set_nonblocking(true).unwrap();
    let tun = Utun::from_owned_fd(device.into(), 1280).unwrap();
    let peer = tokio::net::UnixDatagram::from_std(peer).unwrap();
    let mut batch = PacketBatch::default();
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(5),
            tun.recv_batch(&mut batch)
        )
        .await
        .is_err()
    );
    for (family, version) in [(AF_INET, 0x45), (AF_INET6, 0x60)] {
        let mut bytes = family.to_be_bytes().to_vec();
        bytes.extend([version; 40]);
        peer.send(&bytes).await.unwrap();
    }
    tun.recv_batch(&mut batch).await.unwrap();
    assert_eq!(batch.packets(), &[vec![0x45; 40], vec![0x60; 40]]);
    let packets: Vec<_> = batch.packets().iter().map(Vec::as_slice).collect();
    assert_eq!(tun.send_batch(&packets).await.unwrap(), 1);
    let mut bytes = [0; 128];
    assert_eq!(peer.recv(&mut bytes).await.unwrap(), 44);
    assert_eq!(&bytes[..4], &AF_INET.to_be_bytes());
    assert!(peer.try_recv(&mut bytes).is_err());
    assert_eq!(tun.send_batch(&packets[1..]).await.unwrap(), 1);
    assert_eq!(peer.recv(&mut bytes).await.unwrap(), 44);
    assert_eq!(&bytes[..4], &AF_INET6.to_be_bytes());
}

#[test]
fn rejects_short_mismatched_and_oversize_frames() {
    assert!(decode(&[0, 0, 0, 2], 1280).is_err());
    assert!(decode(&[0, 0, 0, 2, 0x60], 1280).is_err());
    assert!(decode(&[0, 0, 0, 30, 0x45], 1280).is_err());
    let mut bytes = AF_INET.to_be_bytes().to_vec();
    bytes.extend(vec![0x45; 1281]);
    assert!(decode(&bytes, 1280).is_err());
}
