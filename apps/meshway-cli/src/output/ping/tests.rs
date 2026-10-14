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
    view.names(&[], None);
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
        mtu: None,
        via: Vec::new(),
        rtt: None,
        idle_path: None,
    };
    view.names(&[peer(R, "meshway-sha")], None);
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

const E: &str = "00000000-0000-4000-8000-000000000005";

fn decision(action: serde_json::Value, rule: serde_json::Value, path: &[&str]) -> PingEvent {
    serde_json::from_value(json!({"type":"decision","decision":{
        "destination":"11.77.0.3","node":null,"resolution":null,"profile":"exit",
        "rule":rule,"action":action,"path":path}}))
    .unwrap()
}

fn echo(outcome: &str, from: &str, error: serde_json::Value, path: &[&str]) -> PingEvent {
    serde_json::from_value(json!({"type":"echo","echo":{"sequence":0,"outcome":outcome,
        "from":from,"error":error,"durationUs":142_310,"setup":false,"path":path}}))
    .unwrap()
}

fn named(events: &[PingEvent]) -> String {
    let mut view = Ping::new(false);
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
        mtu: None,
        via: Vec::new(),
        rtt: None,
        idle_path: None,
    };
    assert!(view.wants_names(&events[0]));
    view.names(
        &[peer(R, "lax"), peer(E, "it7")],
        Some((A.parse().unwrap(), "a".into())),
    );
    let mut output = Vec::new();
    for event in events {
        view.write(&mut output, &Term::plain(), event).unwrap();
    }
    String::from_utf8(output).unwrap()
}

#[test]
fn an_exit_target_prints_its_decision_then_one_line_per_echo_and_the_summary() {
    let path = [A, R, E];
    let text = named(&[
        decision(json!({"type":"exit","node":E}), json!(null), &path),
        echo("reply", "11.77.0.3", json!(null), &path),
        echo(
            "unreachable",
            "11.77.0.1",
            json!({"kind":3,"code":1}),
            &path,
        ),
        PingEvent::Completed {
            sent: 3,
            success: 1,
            timeout: 1,
            failed: 0,
        },
    ]);
    assert_eq!(
        text,
        "decision for 11.77.0.3: exit it7 via a → lax → it7 · profile exit, fallback\n\
         pong from 11.77.0.3 via exit it7 (a → lax → it7) in 142.31 ms\n\
         unreachable from 11.77.0.1 (host unreachable) via exit it7 (a → lax → it7) in 142.31 ms\n\
         11.77.0.3: 3 sent, 1 replied, 33% loss, 1 unreachable · rtt min/avg/max 142.31/142.31/142.31 ms\n"
    );
}

#[test]
fn a_name_resolved_through_the_exit_shows_its_association_and_rule() {
    let event: PingEvent = serde_json::from_value(json!({"type":"decision","decision":{
        "destination":"11.77.0.3","node":null,
        "resolution":{"name":"svc.name.exit-dns.test","resolver":{"type":"exit","node":E},
            "addresses":["11.77.0.3"],"association":{"names":["svc.name.exit-dns.test"],"remainingSeconds":58}},
        "profile":"exit",
        "rule":{"pointer":"/routing/egress/profiles/exit/rules/2","remark":"names through E","ruleSet":null},
        "action":{"type":"exit","node":E},"path":[A,R,E]}}))
    .unwrap();
    assert_eq!(
        named(&[event]),
        "svc.name.exit-dns.test resolved via exit it7: 11.77.0.3 · associated as svc.name.exit-dns.test, 58 s left\n\
         decision for 11.77.0.3: exit it7 via a → lax → it7 · profile exit, rule /routing/egress/profiles/exit/rules/2 \"names through E\"\n"
    );
}

#[test]
fn direct_unmanaged_and_denied_decisions_say_so_and_denied_is_not_probed() {
    let rule = json!({"pointer":"/routing/egress/profiles/exit/rules/0","remark":"split",
        "ruleSet":{"key":"exit-direct","entry":"192.168.5.3/32"}});
    let text = lines(
        &[
            decision(json!({"type":"direct"}), rule, &[]),
            echo("reply", "11.77.0.3", json!(null), &[]),
        ],
        false,
        &Term::plain(),
    );
    assert_eq!(
        text,
        "decision for 11.77.0.3: direct, not via Mesh · profile exit, rule /routing/egress/profiles/exit/rules/0 \"split\" (rule set exit-direct: 192.168.5.3/32)\n\
         pong from 11.77.0.3 direct, not via Mesh in 142.31 ms\n"
    );
    let text = lines(
        &[
            decision(json!({"type":"unmanaged"}), json!(null), &[]),
            echo("timeout", "11.77.0.3", json!(null), &[]),
        ],
        false,
        &Term::plain(),
    );
    assert!(
        text.starts_with(
            "decision for 11.77.0.3: unmanaged, not via Mesh · profile exit, fallback\n"
        )
    );
    assert!(text.ends_with("no reply from 11.77.0.3 unmanaged, not via Mesh within 5 s\n"));
    let text = lines(
        &[
            decision(
                json!({"type":"denied","reason":"unauthorized"}),
                json!(null),
                &[],
            ),
            PingEvent::Failed {
                code: meshway_protocol::ipc::PingError::PermissionDenied,
            },
        ],
        false,
        &Term::plain(),
    );
    assert_eq!(
        text,
        "decision for 11.77.0.3: denied (unauthorized), not probed · profile exit, fallback\n\
         ping failed: permission denied\n"
    );
    // An unresolved name has no decision beyond its resolution.
    let unresolved: PingEvent = serde_json::from_value(json!({"type":"decision","decision":{
        "destination":null,"node":null,"resolution":{"name":"nx.example","resolver":{"type":"system"},
        "addresses":[],"association":null},"profile":null,"rule":null,"action":null,"path":[]}}))
    .unwrap();
    assert_eq!(
        lines(&[unresolved], false, &Term::plain()),
        "nx.example did not resolve via system DNS\n"
    );
}

#[test]
fn a_mesh_target_decision_shows_only_verbose_and_json_keeps_the_decision_first() {
    let mesh: PingEvent = serde_json::from_value(json!({"type":"decision","decision":{
        "destination":"100.64.0.6","node":{"nodeId":B,"hostname":"meshway-nyc","addresses":{"ipv4":"100.64.0.6","ipv6":null}},
        "resolution":null,"profile":null,"rule":null,"action":{"type":"mesh"},"path":[A,B]}}))
    .unwrap();
    assert_eq!(text(&mesh), "");
    let verbose = lines(&[mesh.clone(), accepted(), sample()], true, &Term::plain());
    assert!(verbose.starts_with(&format!("decision for meshway-nyc: Mesh via {A} → {B}\n")));
    assert!(verbose.contains("pong from meshway-nyc"));
    let events = [mesh, echo("reply", "11.77.0.3", json!(null), &[A, B])];
    let mut output = Vec::new();
    for event in &events {
        json(&mut output, event).unwrap();
    }
    let text = String::from_utf8(output).unwrap();
    let first: serde_json::Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
    assert_eq!(first["type"], "decision");
    assert_eq!(first["decision"]["rule"], serde_json::Value::Null);
    let second: serde_json::Value = serde_json::from_str(text.lines().nth(1).unwrap()).unwrap();
    assert_eq!(second["echo"]["error"], serde_json::Value::Null);
}
