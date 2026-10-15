//! A host that cannot queue an exit packet drops its wrapper: before or after
//! the projection ACK, the flow keeps its incarnation, no Retire or tombstone
//! appears and the next packet of the tuple is written and answered.
use super::netmap_exit::{deliver_reply, fixture, forward, handshake, udp};
use super::support::netmap::DAY_MS;
use meshway_dataplane::exit::ExitProjection;
use std::time::Duration;

#[tokio::test]
async fn netmap_exit_dropped_packet_keeps_its_flow() {
    let mut f = fixture(DAY_MS, Default::default(), Duration::from_secs(60)).await;
    let id = handshake(&mut f).await;
    let input = udp(&f, false, 41000);
    drop(forward(&mut f, id, input));
    let ExitProjection::Install { flow, ack, .. } = f.b.next_exit_projection().unwrap() else {
        panic!("install")
    };
    ack.complete(true).unwrap();
    for _ in 0..3 {
        let input = udp(&f, false, 41000);
        let dropped = forward(&mut f, id, input);
        assert_eq!(dropped.exit_flow(), Some(flow));
        drop(dropped);
    }
    assert!(f.b.next_exit_projection().is_none());
    assert_eq!(f.b.flow_count(), 1);
    let input = udp(&f, false, 41000);
    let next = forward(&mut f, id, input);
    assert_eq!(next.exit_flow(), Some(flow));
    next.projection_ready().await.unwrap();
    next.release(|_| ()).unwrap();
    let reply = udp(&f, true, 41000);
    deliver_reply(&mut f, id, reply);
    assert_eq!(f.b.flow_count(), 1);
    f.close().await;
}
