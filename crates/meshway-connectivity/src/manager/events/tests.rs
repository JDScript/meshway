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
                protocol: meshway_model::TransportProtocol::NativeUdp,
            },
            local: "127.0.0.1:1".parse().unwrap(),
            remote: "127.0.0.1:2".parse().unwrap(),
            acquisition: LinkAcquisition::PublishedEndpoint,
            peer_boot: [3; 16],
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

#[tokio::test]
async fn batch_drain_preserves_fifo_and_partial_group_storage_across_publications() {
    let (tx, mut rx) = channel(5, 40);
    let mut original = Vec::new();
    let mut publish = |count| {
        (0..count)
            .map(|_| {
                let event = frame(8);
                let Event::Frame { frame, .. } = &event else {
                    unreachable!()
                };
                original.push(frame.payload.as_ptr());
                event
            })
            .collect()
    };
    tx.try_send_batch(publish(2)).unwrap();
    tx.try_send_batch(publish(3)).unwrap();
    let mut events = Vec::new();
    assert_eq!(rx.recv_many(&mut events, 3).await, 3);
    assert_eq!(tx.capacity(), 2);
    assert_eq!(tx.bytes.available_permits(), 16);
    rx.close();
    assert_eq!(rx.recv_many(&mut events, 32).await, 2);
    for (event, pointer) in events.iter().zip(original) {
        let Event::Frame { frame, .. } = event else {
            unreachable!()
        };
        assert_eq!(frame.payload.as_ptr(), pointer);
    }
    assert_eq!(tx.capacity(), 5);
    assert_eq!(tx.bytes.available_permits(), 40);
    assert_eq!(rx.recv_many(&mut events, 32).await, 0);
}

#[tokio::test]
async fn a_reservation_waits_for_the_consumer_holds_its_room_and_fits_only_its_group() {
    let (tx, mut rx) = channel(4, 64);
    tx.try_send_batch(vec![frame(8), frame(8), frame(8)])
        .unwrap();
    let group = vec![frame(8), frame(8)];
    let mut room = Box::pin(tx.reserve(&group));
    assert!(futures_util::poll!(room.as_mut()).is_pending());
    let mut events = Vec::new();
    assert_eq!(rx.recv_many(&mut events, 1).await, 1);
    // Room returns only once the whole earlier group is dequeued.
    assert!(futures_util::poll!(room.as_mut()).is_pending());
    events.clear();
    assert_eq!(rx.recv_many(&mut events, 32).await, 2);
    let reservation = room.await.unwrap();
    assert_eq!(tx.capacity(), 2);
    // The held room is not taken by another publisher.
    tx.try_send_batch(vec![frame(8), frame(8)]).unwrap();
    assert!(matches!(
        tx.try_send(frame(8)),
        Err(mpsc::error::TrySendError::Full(_))
    ));
    tx.send_reserved(reservation, group).unwrap();
    assert_eq!(tx.capacity(), 0);
    let other = tx.reserve(&[frame(8)]);
    rx.close();
    assert!(other.await.is_none(), "a closed queue ends the wait");
}
