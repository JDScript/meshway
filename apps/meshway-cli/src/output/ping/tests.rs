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
                "path":[A,B],"publicationId":A,"startedAt":"2026-10-05T00:00:00Z",
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

fn text(event: &PingEvent) -> String {
    let mut output = Vec::new();
    write_event(&mut output, event, false).unwrap();
    String::from_utf8(output).unwrap()
}

#[test]
fn direct_reports_measured_rtt_channel_and_stun_mapping_source() {
    let text = text(&sample());
    assert!(text.contains("RTT=5.123 ms"));
    assert!(text.contains("direct remote=198.51.100.2:32000 transport=native_udp"));
    assert!(text.contains("acquisition=coordinatedUdp endpointSource=STUN (mapped endpoint)"));
    assert!(!text.contains("1000.000"));
}

#[test]
fn via_is_ordered_and_never_labels_whole_path_with_first_hop_transport() {
    let mut event = sample();
    if let PingEvent::Sample { sample, .. } = &mut event {
        sample.probe.as_mut().unwrap().path = [A, R, S, B].map(|id| id.parse().unwrap()).to_vec();
    }
    let text = text(&event);
    assert!(text.contains(&format!("via {R} -> {S}")));
    assert!(
        !text.contains("direct remote=")
            && !text.contains("native_udp")
            && !text.contains("198.51.100.2")
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
        assert!(!text.contains("0.000") && !text.contains("direct remote="));
        let expected = match outcome {
            ProbeOutcome::Success => "RTT=unavailable",
            ProbeOutcome::Timeout => "timeout",
            _ => "local-failed",
        };
        assert!(text.contains(expected));
        if outcome != ProbeOutcome::Success {
            assert!(!text.contains("RTT="));
        }
    }
    if let PingEvent::Sample { sample, .. } = &mut event {
        sample.probe = None;
    }
    assert!(text(&event).contains("no probe data"));
}

#[test]
fn accepted_addresses_preparation_and_summary_are_distinct_lines() {
    let accepted: PingEvent = serde_json::from_value(json!({"type":"accepted","subscriptionId":A,"observationId":A,"target":{"nodeId":B,"hostname":"peer.mesh","addresses":{"ipv4":"10.0.0.2","ipv6":"fd00::2"}}})).unwrap();
    assert!(text(&accepted).contains("peer.mesh"));
    assert!(text(&accepted).contains("10.0.0.2 fd00::2"));
    let preparing = PingEvent::Preparing {
        elapsed_ms: meshway_model::Sequence::new(1523).unwrap(),
    };
    assert_eq!(text(&preparing), "preparing: elapsed=1523 ms\n");
    assert_eq!(
        text(&PingEvent::Completed {
            sent: 10,
            success: 7,
            timeout: 2,
            failed: 1
        }),
        "sent=10 replied=7 timeout=2 local-failed=1\n"
    );
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
        write_event(&mut output, event, true).unwrap();
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
