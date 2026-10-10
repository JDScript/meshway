use super::*;

#[test]
fn staggered_expiry_keeps_refreshed_reverse_flows_and_reclaims_capacity_on_both_clocks() {
    let mut scene = Scene::new(BusinessProtocol::Udp, false, true);
    for table in scene.tables.values_mut() {
        table.limits.max_flows = 256;
        table.limits.idle_timeout = Duration::from_secs(2);
    }
    for port in 50000..50256 {
        forward(&mut scene, udp(1, 3, port, 443));
    }
    scene.advance(Duration::from_secs(1));
    // Repeated receive-only refresh must replace old deadlines, including for
    // tuples whose canonical lookup direction differs from the local packet.
    for _ in 0..3 {
        for port in (50000..50256).step_by(2) {
            reply(&mut scene, udp(3, 1, 443, port));
        }
    }
    scene.advance(Duration::from_secs(1));
    for id in [node(1), node(3)] {
        let table = scene.tables.get_mut(&id).unwrap();
        table.expire(scene.instant, scene.now_ms);
        assert_eq!(table.len(), 128);
    }
    for port in 50000..50256 {
        let result = scene.process(3, None, 3, &[1, 2, 3], udp(3, 1, 443, port));
        assert_eq!(result.is_ok(), port % 2 == 0);
    }
    for port in (50000..50256).step_by(2) {
        reply(&mut scene, udp(3, 1, 443, port));
    }
    for port in (50001..50256).step_by(2) {
        forward(&mut scene, udp(1, 3, port, 443));
    }
    for id in [node(1), node(3)] {
        let table = scene.tables.get_mut(&id).unwrap();
        assert_eq!(table.len(), 256);
        // Idle expiry collects every tuple once no packet refreshed it. The
        // netmap lease bounds flows at use; it is never copied into them.
        table.expire(scene.instant + Duration::from_secs(3), scene.now_ms + 3000);
        assert_eq!(table.len(), 0);
    }
    forward(&mut scene, udp(1, 3, 50000, 443));
    reply(&mut scene, udp(3, 1, 443, 50000));
    for table in scene.tables.values_mut() {
        table.clear();
        assert_eq!(table.len(), 0);
    }
    forward(&mut scene, udp(1, 3, 50000, 443));
}
