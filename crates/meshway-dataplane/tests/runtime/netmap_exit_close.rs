//! An exit cipher that ends with its last flow closes toward its origin:
//! the relay forwards the Close and the origin's next exit cipher is new.
use super::netmap_exit::{deliver_reply, fixture, forward, handshake, udp};
use super::support::netmap::DAY_MS;
use meshway_dataplane::{exit::ExitProjection, secure::Received};
use std::time::Duration;
#[tokio::test]
async fn netmap_exit_cipher_ending_with_its_last_flow_closes_toward_the_origin() {
    let mut f = fixture(DAY_MS, Default::default(), Duration::from_secs(1)).await;
    let [a, r, _] = f.ids();
    let id = handshake(&mut f).await;
    let input = udp(&f, false, 41000);
    let first = forward(&mut f, id, input);
    let ExitProjection::Install { scope, ack, .. } = f.b.next_exit_projection().unwrap() else {
        panic!("install")
    };
    ack.complete(true).unwrap();
    first.release(|_| ()).unwrap();
    let reply = udp(&f, true, 41000);
    deliver_reply(&mut f, id, reply);
    assert!(f.b.take_exit_closes().is_empty());
    // The last flow idles out: the exit ends the cipher and owes the origin
    // a Close, relayed by R, so A's next packet opens a new cipher at once.
    tokio::time::sleep_until((scope.deadline() + Duration::from_millis(10)).into()).await;
    assert!(matches!(
        f.b.next_exit_projection(),
        Some(ExitProjection::Retire { .. })
    ));
    let mut closes = f.b.take_exit_closes();
    assert_eq!(closes.len(), 1);
    let close = closes.remove(0);
    assert_eq!((close.next(), close.circuit()), (r, id));
    let Received::Forward(relayed) =
        f.r.receive(&f.rb.carrier, super::netmap_flow::wire(close))
            .unwrap()
    else {
        panic!("relayed Close")
    };
    assert_eq!(relayed.next(), a);
    assert!(
        f.r.circuit_info(id).unwrap().is_none(),
        "the relay ended it"
    );
    // A ends its own end with its last flow too (here at the same instant);
    // with a later last flow the relayed Close ends it, as for a restarted
    // terminal. A live origin circuit closes on it.
    match f
        .a
        .receive(&f.ar.carrier, super::netmap_flow::wire(relayed))
    {
        Ok(Received::Closed { circuit }) => assert_eq!(circuit, id),
        Ok(_) => panic!("a Close only closes"),
        Err(_) => assert!(f.a.circuit_info(id).unwrap().is_none()),
    }
    // The next exit cipher is a new one.
    let again = handshake(&mut f).await;
    assert_ne!(again, id);
    f.close().await;
}
