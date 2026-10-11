use super::*;
use serde_json::json;

const A: &str = "00000000-0000-4000-8000-000000000001";
const B: &str = "00000000-0000-4000-8000-000000000002";
const R: &str = "00000000-0000-4000-8000-000000000003";
const S: &str = "00000000-0000-4000-8000-000000000004";

fn sample() -> PingEvent {
    serde_json::from_value(json!({
        "type":"sample","observationId":A,
        "sample":{
            "measurementId":A,"version":1,"windowStart":"2026-10-05T00:00:00Z",
            "windowEnd":"2026-10-05T00:00:01Z","partial":false,"traffic":null,"probeCounts":null,
            "probe":{
                "measurementId":A,"kind":"path","linkId":null,"transport":null,
                "path":[A,B],"netmap":{"revision":1,"serial":1,"digest":"01".repeat(32)},"startedAt":"2026-10-05T00:00:00Z",
                "completedAt":"2026-10-05T00:00:01Z","outcome":"success","durationUs":5123,
                "channels":{"request":{
                    "linkId":"01".repeat(32),"socketId":"02".repeat(32),"peerNodeId":B,
                    "transport":"native_udp","localAddress":"192.0.2.1:31000","remoteAddress":"198.51.100.2:32000",
                    "acquisition":{"type":"coordinatedUdp","attemptId":A,"endpointSource":"stun"}
                },"reply":null}
            }
        }
    })).unwrap()
}

fn accepted() -> PingEvent {
    serde_json::from_value(json!({"type":"accepted","subscriptionId":A,"observationId":A,"target":{"nodeId":B,"hostname":"meshway-nyc","addresses":{"ipv4":"100.64.0.6","ipv6":"fd00::6"}}})).unwrap()
}

fn lines(events: &[PingEvent], verbose: bool, term: &Term) -> String {
    let mut view = Ping::new(verbose);
    view.names(&[]);
    let mut output = Vec::new();
    for event in events {
        view.write(&mut output, term, event).unwrap();
    }
    String::from_utf8(output).unwrap()
}

fn text(event: &PingEvent) -> String {
    lines(&[accepted(), event.clone()], false, &Term::plain())
}

#[test]
fn direct_is_one_line_with_peer_channel_and_exact_rtt() {
    assert_eq!(
        text(&sample()),
        "pong from meshway-nyc (100.64.0.6) via 198.51.100.2:32000 native_udp in 5.12 ms\n"
    );
    let verbose = lines(&[accepted(), sample()], true, &Term::plain());
    assert!(verbose.starts_with("PING meshway-nyc (100.64.0.6)"));
    assert!(verbose.contains("[coordinated UDP, STUN-mapped endpoint]"));
}

#[test]
fn relays_are_named_from_peers_and_the_first_hop_channel_is_labelled_as_such() {
    let mut event = sample();
    if let PingEvent::Sample { sample, .. } = &mut event {
        sample.probe.as_mut().unwrap().path = [A, R, S, B].map(|id| id.parse().unwrap()).to_vec();
    }
    let mut view = Ping::new(false);
    view.write(&mut Vec::new(), &Term::plain(), &accepted())
        .unwrap();
    assert!(view.wants_names(&event));
    let peer = |id: &str, name: &str| meshway_protocol::ipc::PeerStatus {
        node_id: id.parse().unwrap(),
        hostname: name.into(),
        addresses: meshway_configuration::NodeAddresses {
            ipv4: None,
            ipv6: None,
        },
        state: meshway_protocol::ipc::PeerState::Direct,
        transport: None,
        endpoint: None,
        via: Vec::new(),
        rtt: None,
    };
    view.names(&[peer(R, "meshway-sha")]);
    assert!(!view.wants_names(&event));
    let mut output = Vec::new();
    view.write(&mut output, &Term::plain(), &event).unwrap();
    assert_eq!(
        String::from_utf8(output).unwrap(),
        format!(
            "pong from meshway-nyc (100.64.0.6) via relay meshway-sha → {S} (198.51.100.2:32000 native_udp) in 5.12 ms\n"
        )
    );
}

#[test]
fn missing_measurement_timeout_and_local_failure_never_invent_zero_rtt() {
    let mut event = sample();
    for outcome in [
        ProbeOutcome::Success,
        ProbeOutcome::Timeout,
        ProbeOutcome::Failed,
    ] {
        if let PingEvent::Sample { sample, .. } = &mut event {
            let probe = sample.probe.as_mut().unwrap();
            probe.outcome = outcome;
            probe.duration_us = None;
            probe.channels = None;
        }
        let text = text(&event);
        assert!(!text.contains("0.00"));
        let expected = match outcome {
            ProbeOutcome::Success => "with no measured round trip",
            ProbeOutcome::Timeout => "no reply from meshway-nyc (100.64.0.6) within 5 s",
            _ => "probe to meshway-nyc (100.64.0.6) failed locally",
        };
        assert!(text.contains(expected), "{text}");
    }
    if let PingEvent::Sample { sample, .. } = &mut event {
        sample.probe = None;
    }
    assert!(text(&event).contains("without probe data"));
}

#[test]
fn preparing_is_verbose_only_and_the_summary_closes_with_loss_and_rtt() {
    let preparing = PingEvent::Preparing {
        elapsed_ms: meshway_model::Sequence::new(1523).unwrap(),
    };
    assert_eq!(text(&preparing), "");
    assert_eq!(
        lines(&[accepted(), preparing.clone()], true, &Term::plain())
            .lines()
            .last(),
        Some("preparing (1523 ms)")
    );
    let completed = PingEvent::Completed {
        sent: 10,
        success: 7,
        timeout: 2,
        failed: 1,
    };
    assert_eq!(
        lines(&[accepted(), sample(), completed], false, &Term::plain())
            .lines()
            .last(),
        Some(
            "meshway-nyc: 10 sent, 7 replied, 20% loss, 1 failed locally · rtt min/avg/max 5.12/5.12/5.12 ms"
        )
    );
    let colored = lines(&[accepted(), sample()], false, &Term::rich());
    assert!(colored.contains('\u{1b}') && colored.contains("pong"));
}

#[test]
fn json_lines_are_exact_authoritative_events_including_nulls() {
    let events = [
        sample(),
        PingEvent::Completed {
            sent: 1,
            success: 1,
            timeout: 0,
            failed: 0,
        },
    ];
    let mut output = Vec::new();
    for event in &events {
        json(&mut output, event).unwrap();
    }
    let text = String::from_utf8(output).unwrap();
    assert_eq!(text.lines().count(), 2);
    for (line, expected) in text.lines().zip(events) {
        let decoded: PingEvent = serde_json::from_str(line).unwrap();
        assert_eq!(
            serde_json::to_value(decoded).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
    }
    assert!(text.contains("\"reply\":null"));
}
