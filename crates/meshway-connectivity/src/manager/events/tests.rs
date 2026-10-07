use super::*;
use crate::{Carrier, LinkFact};
use meshway_model::observations::LinkAcquisition;

fn frame(capacity: usize) -> Event {
    let peer = "11111111-1111-4111-8111-111111111111".parse().unwrap();
    let mut bytes = Vec::with_capacity(capacity);
    bytes.push(7);
    Event::Frame {
        link: Box::new(LinkFact {
            id: [1; 32],
            socket_id: [2; 32],
            peer,
            carrier: Carrier {
                dialer: peer,
                listener: peer,
                listener_key: "test".into(),
                protocol: meshway_transport_spec::TransportProtocol::NativeUdp,
            },
            local: "127.0.0.1:1".parse().unwrap(),
            remote: "127.0.0.1:2".parse().unwrap(),
            acquisition: LinkAcquisition::PublishedEndpoint,
        }),
        frame: meshway_transport::Frame::new(bytes, None),
        received_at: std::time::Instant::now(),
    }
}

#[tokio::test]
async fn storage_capacity_bounds_small_payloads_and_dequeue_returns_original_allocation() {
    let (tx, mut rx) = channel(4, 8);
    let event = frame(8);
    let Event::Frame { frame: data, .. } = &event else {
        unreachable!()
    };
    let pointer = data.payload.as_ptr();
    tx.try_send(event).unwrap();
    assert_eq!(tx.capacity(), 3);
    assert!(matches!(
        tx.try_send(frame(1)),
        Err(mpsc::error::TrySendError::Full(_))
    ));
    let mut events = Vec::new();
    assert_eq!(rx.recv_many(&mut events, 32).await, 1);
    let Event::Frame { frame: data, .. } = &events[0] else {
        unreachable!()
    };
    assert_eq!(data.payload.as_ptr(), pointer);
    assert_eq!(tx.bytes.available_permits(), 8);
    tx.try_send(frame(8)).unwrap();
    drop(rx);
    assert_eq!(tx.bytes.available_permits(), 8);
}

#[tokio::test]
async fn closing_receiver_wakes_storage_waiter_without_reclaiming_queued_payload_early() {
    let (tx, mut rx) = channel(4, 8);
    tx.try_send(frame(8)).unwrap();
    let mut send = Box::pin(tx.send(frame(8)));
    assert!(futures_util::poll!(send.as_mut()).is_pending());
    rx.close();
    assert!(send.await.is_err());
    assert_eq!(tx.bytes.available_permits(), 0);
    rx.try_recv().unwrap();
    assert_eq!(tx.bytes.available_permits(), 8);
}

#[tokio::test]
async fn partial_batch_dequeue_keeps_suffix_bounded_and_close_drains_in_order() {
    let (tx, mut rx) = channel(3, 24);
    tx.try_send_batch(vec![frame(8), frame(8), frame(8)])
        .unwrap();
    let mut events = Vec::new();
    assert_eq!(rx.recv_many(&mut events, 1).await, 1);
    assert!(matches!(
        tx.try_send(frame(1)),
        Err(mpsc::error::TrySendError::Full(_))
    ));
    assert_eq!(tx.bytes.available_permits(), 0);
    rx.close();
    assert_eq!(rx.recv_many(&mut events, 1).await, 1);
    assert_eq!(rx.recv_many(&mut events, 32).await, 1);
    assert_eq!(events.len(), 3);
    assert_eq!(tx.bytes.available_permits(), 24);
    assert_eq!(tx.capacity(), 3);
    assert_eq!(rx.recv_many(&mut events, 32).await, 0);
}
