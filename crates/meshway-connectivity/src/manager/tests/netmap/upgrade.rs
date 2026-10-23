//! A resident edge whose stream carrier was acquired while its earlier Native
//! UDP carrier was unavailable returns to Native UDP once that is
//! acquirable, on the candidate back-off and beside the stream.
use super::*;

/// a ↔ b resident, carriers in the platform order (Native UDP, Native TCP);
/// b listens on both. Only a dials: b's dials fail.
async fn udp_then_tcp(fixture: &mut Netmaps) -> (NodeId, NodeId) {
    let ids = fixture.ids();
    let (a_id, b_id) = (ids[0], ids[1]);
    let tcp = fixture.tcp_address();
    fixture
        .publish(|cfg| {
            cfg.topology.push(TopologyRule {
                remark: String::new(),
                action: TopologyAction::Allow,
                a: format!("node:{a_id}"),
                b: format!("node:{b_id}"),
                transports: meshway_configuration::Input::Omitted,
                keepalive: true,
            });
            cfg.defaults.dialers.native_tcp = Some(Default::default());
            cfg.nodes.get_mut(&b_id).unwrap().listeners.insert(
                "tcp".into(),
                meshway_transport::spec::PreparedListener::prepare(support::tcp_listener(tcp))
                    .unwrap(),
            );
        })
        .await;
    (a_id, b_id)
}

fn upgrade_limits() -> Limits {
    Limits {
        retry_backoff: Duration::from_millis(100),
        ..limits(Duration::from_secs(60))
    }
}

/// Both ends hold the same channel on `protocol`.
fn converged(
    a: &Manager,
    b: &Manager,
    a_id: NodeId,
    b_id: NodeId,
    protocol: TransportProtocol,
) -> bool {
    let (a, b) = (links(a), links(b));
    a.get(&b_id).is_some_and(|link| {
        link.carrier.protocol == protocol && b.get(&a_id).is_some_and(|back| back.id == link.id)
    })
}

/// Native UDP fails, so Native TCP is selected; once Native UDP is
/// acquirable again the edge returns to it within the candidate back-off
/// (here 100 ms doubling) plus a handshake, replacing exactly the TCP
/// channel. Before CM-1b a selected channel stopped every other candidate:
/// the edge stayed on TCP for the channel's life.
#[tokio::test]
async fn a_stream_selected_during_a_udp_outage_returns_to_native_udp() {
    let mut fixture = Netmaps::start(false, true).await;
    let (a_id, b_id) = udp_then_tcp(&mut fixture).await;
    let factory = fixture.factory();
    factory.blocked_dialers.lock().unwrap().insert(b_id);
    factory.blocked_datagrams.lock().unwrap().insert(a_id);
    let mut a = manager(&fixture, 0, &factory, upgrade_limits()).await;
    let mut b = manager(&fixture, 1, &factory, upgrade_limits()).await;
    eventually("Native TCP is selected while Native UDP fails", || {
        converged(&a, &b, a_id, b_id, TransportProtocol::NativeTcp)
    })
    .await;
    let tcp = links(&a)[&b_id].id;
    // A while on TCP: the attempts beside it back off and leave it as it is.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(
        links(&a)[&b_id].id,
        tcp,
        "a failed attempt keeps the stream"
    );
    factory.blocked_datagrams.lock().unwrap().clear();
    let restored = tokio::time::Instant::now();
    eventually("the edge returns to Native UDP", || {
        converged(&a, &b, a_id, b_id, TransportProtocol::NativeUdp)
    })
    .await;
    // The pause had grown to at most 1.6 s (0.1 · 2⁴) after 1.5 s of failures.
    assert!(
        restored.elapsed() < Duration::from_secs(5),
        "{:?}",
        restored.elapsed()
    );
    // The new channel stays: nothing ranks before it.
    let udp = links(&a)[&b_id].id;
    let dials = factory.datagram_dials.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(links(&a)[&b_id].id, udp);
    assert_eq!(links(&b)[&a_id].id, udp);
    assert_eq!(factory.datagram_dials.load(Ordering::SeqCst), dials);
    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
    fixture.close().await;
}

/// Native UDP never becomes acquirable: the attempts beside the selected TCP
/// channel follow the candidate back-off (about 0.1, 0.3, 0.7, 1.5, 3.1 and
/// 6.3 s after the first), never touching the channel.
#[tokio::test]
async fn an_unreachable_earlier_udp_is_tried_on_the_back_off_and_the_stream_stays() {
    let mut fixture = Netmaps::start(false, true).await;
    let (a_id, b_id) = udp_then_tcp(&mut fixture).await;
    let factory = fixture.factory();
    factory.blocked_dialers.lock().unwrap().insert(b_id);
    factory.blocked_datagrams.lock().unwrap().insert(a_id);
    let mut a = manager(&fixture, 0, &factory, upgrade_limits()).await;
    let mut b = manager(&fixture, 1, &factory, upgrade_limits()).await;
    eventually("Native TCP is selected", || {
        converged(&a, &b, a_id, b_id, TransportProtocol::NativeTcp)
    })
    .await;
    let tcp = links(&a)[&b_id].id;
    let start = tokio::time::Instant::now();
    let first = factory.datagram_dials.load(Ordering::SeqCst);
    tokio::time::sleep_until(start + Duration::from_secs(7)).await;
    let attempts = factory.datagram_dials.load(Ordering::SeqCst) - first;
    // A fixed 100 ms pace would be ~70; the doubling back-off allows ≤ 9.
    assert!((4..=9).contains(&attempts), "{attempts} attempts in 7 s");
    assert_eq!(links(&a)[&b_id].id, tcp);
    assert_eq!(links(&b)[&a_id].id, tcp);
    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
    fixture.close().await;
}
