use super::*;
use std::time::Duration;

fn key(host: &str) -> TargetKey {
    (
        "peer/udp".to_owned(),
        host.to_owned(),
        meshway_transport::spec::Port::new(35000).unwrap(),
    )
}

#[test]
fn a_failing_candidate_doubles_its_pause_up_to_one_minute() {
    let now = tokio::time::Instant::now();
    let (private, public) = (key("192.168.139.66"), key("198.51.100.7"));
    let mut pacing = Pacing::default();
    pacing.sweep(
        crate::plan::Edge::default(),
        [private.clone(), public.clone()].into(),
        Default::default(),
        0,
        None,
    );
    let base = Duration::from_secs(1);
    let pauses: Vec<_> = (0..11)
        .map(|_| pacing.failed(private.clone(), base, now).1.as_secs())
        .collect();
    assert_eq!(pauses, [1, 2, 4, 8, 16, 32, 60, 60, 60, 60, 60]);
    assert!(!pacing.due(&private, now + Duration::from_secs(59)));
    assert!(pacing.due(&private, now + MAX_CANDIDATE_BACKOFF));
    // Every other candidate, whatever its address class, keeps its own pace.
    assert!(pacing.due(&public, now));
    assert_eq!(pacing.next_due(), None, "an untried candidate is due now");
    pacing.failed(public.clone(), base, now);
    assert_eq!(pacing.next_due(), Some(now + base));
}

#[test]
fn facts_own_mappings_or_a_channel_start_every_candidate_over() {
    let now = tokio::time::Instant::now();
    let base = Duration::from_secs(1);
    let (old, new) = (key("192.168.139.66"), key("203.0.113.9"));
    let failed = |pacing: &mut Pacing| {
        for _ in 0..5 {
            pacing.failed(old.clone(), base, now);
        }
        assert!(!pacing.due(&old, now + base));
    };
    let mut pacing = Pacing::default();
    pacing.sweep(
        crate::plan::Edge::default(),
        [old.clone()].into(),
        Default::default(),
        0,
        None,
    );
    failed(&mut pacing);
    // The same edge, candidates and channel keep the pause (the edge
    // itself is covered with real contents by the Manager tests).
    pacing.sweep(
        crate::plan::Edge::default(),
        [old.clone()].into(),
        Default::default(),
        0,
        None,
    );
    assert!(!pacing.due(&old, now + base));
    // The peer's endpoint facts changed.
    pacing.sweep(
        crate::plan::Edge::default(),
        [old.clone(), new.clone()].into(),
        Default::default(),
        0,
        None,
    );
    assert!(pacing.due(&old, now));
    failed(&mut pacing);
    // This Node's own mapping changed (another egress address).
    pacing.sweep(
        crate::plan::Edge::default(),
        [old.clone(), new.clone()].into(),
        ["203.0.113.9:40001".to_owned()].into(),
        0,
        None,
    );
    assert!(pacing.due(&old, now));
    failed(&mut pacing);
    // This Node's underlay path changed (it roamed).
    assert_eq!(
        pacing.sweep(
            crate::plan::Edge::default(),
            [old.clone(), new.clone()].into(),
            ["203.0.113.9:40001".to_owned()].into(),
            1,
            None,
        ),
        Some("own_network")
    );
    assert!(pacing.due(&old, now));
    failed(&mut pacing);
    // A channel was installed (and may have ended) since the last sweep.
    pacing.sweep(
        crate::plan::Edge::default(),
        [old.clone(), new.clone()].into(),
        Default::default(),
        0,
        Some([7; 32]),
    );
    assert!(pacing.due(&old, now));
    failed(&mut pacing);
    // A selected channel exists now.
    pacing.connected(Some([7; 32]));
    assert!(pacing.due(&old, now));
}
