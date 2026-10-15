//! Exit readiness is a selection fact with last-known semantics: an exit
//! whose fact is past its freshness bound but was last known ready still
//! takes new exit flows the netmap allows; an exit never known ready does not.
use super::netmap_exit::{fixture, udp};
use super::support::netmap::DAY_MS;
use meshway_configuration::execution::UnavailableReason;
use meshway_dataplane::secure::Outbound;
use std::time::Duration;

#[tokio::test]
async fn netmap_exit_a_last_known_ready_exit_takes_new_flows() {
    let mut f = fixture(DAY_MS, Default::default(), Duration::from_secs(60)).await;
    let b = f.ids()[2];
    // Never known ready: refused locally.
    let packet = udp(&f, false, 41000);
    assert!(matches!(
        f.a.outbound(packet).unwrap(),
        Outbound::Denied(UnavailableReason::ExitUnavailable)
    ));
    // Its fact went stale (no sync during the lease): still the choice.
    f.a.set_ready_exits(Default::default(), [b].into());
    let packet = udp(&f, false, 41000);
    let Outbound::Queued { init: Some(init) } = f.a.outbound(packet).unwrap() else {
        panic!("exit circuit to the last known ready exit")
    };
    f.finish_handshake(init).await;
    f.close().await;
}
