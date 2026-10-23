use super::*;
use serde_json::json;

const NYC: &str = "00000000-0000-4000-8000-000000000002";
const SHA: &str = "00000000-0000-4000-8000-000000000003";
const OLD: &str = "00000000-0000-4000-8000-000000000004";
const PLA: &str = "00000000-0000-4000-8000-000000000005";
const VM: &str = "00000000-0000-4000-8000-000000000006";
const IT7: &str = "00000000-0000-4000-8000-000000000007";

fn now() -> DateTime<Utc> {
    "2026-10-11T12:00:00Z".parse().unwrap()
}

fn status() -> Status {
    let peer = |id: &str, name: &str, ip: &str, state: &str| {
        json!({"nodeId":id,"hostname":name,"addresses":{"ipv4":ip,"ipv6":null},
            "state":state,"transport":null,"endpoint":null,"mtu":null,"via":[],"rtt":null})
    };
    let mut nyc = peer(NYC, "meshway-nyc", "100.64.0.6", "relay");
    nyc["via"] = json!([SHA]);
    nyc["transport"] = json!("native_udp");
    nyc["rtt"] = json!({"durationUs":310,"observedAt":"2026-10-11T11:59:50Z"});
    let mut sha = peer(SHA, "meshway-sha", "100.64.0.5", "direct");
    sha["transport"] = json!("native_udp");
    sha["endpoint"] = json!("192.168.97.3:35000");
    sha["mtu"] = json!({"path":1444,"inner":1330});
    sha["rtt"] = json!({"durationUs":140,"observedAt":"2026-10-11T11:59:59Z"});
    // Needed, no circuit now, but reachable through a relay with channels.
    let mut planck = peer(PLA, "meshway-pla", "100.64.0.7", "relayIdle");
    planck["via"] = json!([SHA]);
    planck["transport"] = json!("native_udp");
    planck["idlePath"] = json!({"route":"relay","via":[SHA]});
    // No circuit or channel: the path a new circuit would take.
    let mut vm = peer(VM, "meshway-vm", "100.64.0.8", "idle");
    vm["idlePath"] = json!({"route":"direct","via":[]});
    let mut it7 = peer(IT7, "meshway-it7", "100.64.0.10", "unreachable");
    it7["idlePath"] = json!({"route":"none","via":[]});
    let reference = json!({"revision":42,"serial":17,"digest":"01".repeat(32)});
    serde_json::from_value(json!({
        "processId":1,"revision":3,"desiredUp":true,"phase":"running","server":"http://127.0.0.1:8420",
        "observedAt":"2026-10-11T11:57:00Z","controlObservedAt":"2026-10-11T11:59:48Z","enrollmentRequestId":null,"enrollmentUrl":null,
        "identity":null,"authenticated":true,"sync":"connected","lastSyncAt":"2026-10-11T11:59:48Z",
        "syncError":null,"instance":null,
        "netmap":{"netmap":reference,"issuedAt":"2026-10-11T11:00:00Z","expiresAt":"2026-10-12T11:59:00Z","missingArtifacts":[],"artifactProblem":null,
            "capture":{"managedIpv4":4341,"managedIpv6":0,"sinkIpv6":2256,"routes":12680,"unavailable":null},
            "routing":{"requestedProfile":"work","version":2,"appliedProfile":"work","requestedUnavailable":null,
                "defaultProfile":"mesh","locked":false,"profiles":[],"auto":null}},
        "saved":reference,"applied":reference,"dataPlane":"ready","networkMode":"tun",
        "node":{"hostname":"meshway-lax","addresses":{"ipv4":"100.64.0.4","ipv6":null}},
        "peers":[nyc, sha, peer(OLD, "meshway-old", "100.64.0.9", "offline"), planck, vm, it7],
        "sockets":[
            {"resource":{"kind":"listener","key":"native"},"boundAddress":"0.0.0.0:35000",
             "receiveBuffer":8388608,"receiveRequested":4194304,"sendBuffer":4194304,"sendRequested":2097152,"unavailable":null,
             "receiveDrops":0},
            {"resource":{"kind":"listener","key":"native6"},"boundAddress":"[::]:35000",
             "receiveBuffer":425984,"receiveRequested":4194304,"sendBuffer":4194304,"sendRequested":2097152,"unavailable":null,
             "receiveDrops":1832},
            {"resource":{"kind":"listener","key":"held"},"boundAddress":null,"receiveBuffer":null,"receiveRequested":null,
             "sendBuffer":null,"sendRequested":null,"unavailable":"listener held cannot bind 0.0.0.0:35001: Address in use (os error 98)"}
        ],"lastError":null,"lastErrorDetail":null,"retryAt":null
    }))
    .unwrap()
}

fn render(term: &Term) -> String {
    let mut output = Vec::new();
    write(&mut output, term, &status(), now()).unwrap();
    String::from_utf8(output).unwrap()
}

#[test]
fn plain_summary_and_peer_table_have_no_escapes() {
    let text = render(&Term::plain());
    assert!(!text.contains('\u{1b}') && !text.contains('│'));
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], "meshway-lax  100.64.0.4  running");
    assert!(text.contains("  Sync       connected · last sync 12s ago\n"));
    assert!(text.contains("  Netmap     revision 42 · serial 17 · lease 23h 59m left\n"));
    assert!(text.contains("  Egress     4341 IPv4 · 2256 IPv6 sink ranges · 12680 routes\n"));
    assert!(text.contains("  Profile    work (meshway exit)\n"));
    assert!(
        text.contains("  Data plane ready · mode tun · changed 3m 0s ago\n"),
        "{text}"
    );
    assert!(!text.contains("Relay"));
    let header = lines
        .iter()
        .position(|line| line.starts_with("PEER"))
        .unwrap();
    let expected = [
        "PEER         MESH IPV4    STATE         TRANSPORT   ENDPOINT                      MTU        RTT",
        "meshway-nyc  100.64.0.6   relay         native_udp  via meshway-sha                          0.31 ms",
        "meshway-sha  100.64.0.5   direct        native_udp  192.168.97.3:35000            1444/1330  0.14 ms",
        // No measurement is not zero; no idle path from this daemon: nothing shown.
        "meshway-old  100.64.0.9   offline",
        // No channel or circuit: the path a new circuit would take.
        "meshway-pla  100.64.0.7   relay (idle)  native_udp  relay (idle) via meshway-sha",
        "meshway-vm   100.64.0.8   idle                      direct (idle)",
        "meshway-it7  100.64.0.10  unreachable               no path",
    ];
    assert_eq!(lines[header..header + expected.len()], expected);
    let header = lines
        .iter()
        .position(|line| line.starts_with("SOCKET"))
        .unwrap();
    assert_eq!(
        lines[header + 1].split_whitespace().collect::<Vec<_>>(),
        [
            "listener",
            "native",
            "0.0.0.0:35000",
            "8.0",
            "MiB",
            "4.0",
            "MiB",
            "0"
        ]
    );
    assert!(lines[header].ends_with("DROPS"), "{}", lines[header]);
    assert!(lines[header + 2].contains("416 KiB (requested 4.0 MiB, limited)"));
    assert!(lines[header + 2].trim_end().ends_with("1832"));
    assert_eq!(
        lines[header + 3].split_whitespace().collect::<Vec<_>>(),
        ["listener", "held", "unavailable"]
    );
}

#[test]
fn a_terminal_gets_colors_and_a_box_table() {
    let text = render(&Term::rich());
    assert!(text.contains('\u{1b}') && text.contains('╭') && text.contains('│'));
}

#[test]
fn json_keeps_every_status_field_including_peers_and_nulls() {
    let mut output = Vec::new();
    crate::output::json(&mut output, &status()).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(value["peers"].as_array().unwrap().len(), 6);
    assert_eq!(value["peers"][3]["state"], "relayIdle");
    assert_eq!(value["peers"][3]["idlePath"]["route"], "relay");
    // Omitted, not null, where it does not apply.
    assert!(value["peers"][0].get("idlePath").is_none());
    assert_eq!(value["sockets"][1]["receiveDrops"], 1832);
    assert!(value["sockets"][2].get("receiveDrops").is_none());
    assert_eq!(value["node"]["hostname"], "meshway-lax");
    assert!(value["peers"][2]["rtt"].is_null() && value["identity"].is_null());
    let decoded: Status = serde_json::from_value(value).unwrap();
    assert_eq!(decoded.peers, status().peers);
}

/// F6a B10: with a Control problem as `lastError`, the packet plane's own
/// failure and its cause are still shown.
#[test]
fn the_packet_planes_failure_is_shown_beside_a_control_problem() {
    let mut status = status();
    status.last_error = Some(meshway_protocol::ipc::Problem::Credentials);
    status.network_error = Some(meshway_protocol::ipc::NetworkError {
        problem: meshway_protocol::ipc::Problem::Dns,
        detail: Some("DNS resolved_link check failed".into()),
    });
    let mut output = Vec::new();
    write(&mut output, &Term::plain(), &status, now()).unwrap();
    let text = String::from_utf8(output).unwrap();
    assert!(text.contains("  Problem    credentials\n"), "{text}");
    assert!(
        text.contains("  Network    dns\n             DNS resolved_link check failed\n"),
        "{text}"
    );
}

/// H4 (X14-a): a Control relay that cannot listen, or that refuses new
/// connections at its limit, is one line.
#[test]
fn a_control_relay_that_does_not_serve_every_connection_is_shown() {
    let mut status = status();
    status.control_relay = Some(meshway_protocol::ipc::ControlRelay {
        state: meshway_protocol::ipc::ControlRelayState::Retrying,
        reason: "cannot listen: address in use".into(),
        attempts: 3,
        retry_at: Some("2026-10-11T12:00:04Z".parse().unwrap()),
    });
    let mut output = Vec::new();
    write(&mut output, &Term::plain(), &status, now()).unwrap();
    let text = String::from_utf8(output).unwrap();
    assert!(
        text.contains(
            "  Relay      retrying (cannot listen: address in use; attempt 3, retry in 4s)\n"
        ),
        "{text}"
    );
    status.control_relay = Some(meshway_protocol::ipc::ControlRelay {
        state: meshway_protocol::ipc::ControlRelayState::AtCapacity,
        reason: "tasks".into(),
        attempts: 2,
        retry_at: None,
    });
    let mut output = Vec::new();
    write(&mut output, &Term::plain(), &status, now()).unwrap();
    let text = String::from_utf8(output).unwrap();
    assert!(
        text.contains(
            "  Relay      at capacity (tasks limit; 2 refused, current connections kept)\n"
        ),
        "{text}"
    );
    let mut json = Vec::new();
    crate::output::json(&mut json, &status).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&json).unwrap();
    assert_eq!(value["controlRelay"]["state"], "atCapacity");
    assert!(value["controlRelay"]["retryAt"].is_null());
    assert_eq!(value["controlObservedAt"], "2026-10-11T11:59:48Z");
}

/// D5: another link's specific domains beside the DNS fallback are listed
/// with their links, and absent (omitted on the wire) otherwise.
#[test]
fn domains_resolved_by_the_local_network_are_listed() {
    let mut status = status();
    let text = String::from_utf8({
        let mut output = Vec::new();
        write(&mut output, &Term::plain(), &status, now()).unwrap();
        output
    })
    .unwrap();
    assert!(!text.contains("Local DNS"), "{text}");
    let mut json = Vec::new();
    crate::output::json(&mut json, &status).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&json).unwrap();
    assert!(value.get("localDnsDomains").is_none());
    status.local_dns_domains = Some(vec![
        meshway_protocol::ipc::LocalDnsDomain {
            domain: "tko.oraclevcn.com".into(),
            link_index: 2,
            link_name: Some("enp0s6".into()),
        },
        meshway_protocol::ipc::LocalDnsDomain {
            domain: "corp.example".into(),
            link_index: 7,
            link_name: None,
        },
    ]);
    let mut output = Vec::new();
    write(&mut output, &Term::plain(), &status, now()).unwrap();
    let text = String::from_utf8(output).unwrap();
    assert!(
        text.contains(
            "  Local DNS  tko.oraclevcn.com (enp0s6), corp.example (link 7) · resolved by the local network, not the DNS fallback\n"
        ),
        "{text}"
    );
    let mut json = Vec::new();
    crate::output::json(&mut json, &status).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&json).unwrap();
    assert_eq!(
        value["localDnsDomains"][0],
        serde_json::json!({"domain":"tko.oraclevcn.com","linkIndex":2,"linkName":"enp0s6"})
    );
    let decoded: Status = serde_json::from_value(value).unwrap();
    assert_eq!(decoded.local_dns_domains, status.local_dns_domains);
}

/// H4 decision 1: a packet plane withdrawn for a failure that recovers by
/// itself reads `retrying` (not the operator's `blocked`) with its cause.
#[test]
fn a_recovering_failed_plane_is_retrying_with_its_cause() {
    let mut status = status();
    status.phase = Phase::Retrying;
    status.data_plane = DataPlane::Failed;
    status.applied = None;
    status.last_error = Some(meshway_protocol::ipc::Problem::Dns);
    status.last_error_detail = Some("DNS observe_deadline check failed".into());
    let mut output = Vec::new();
    write(&mut output, &Term::plain(), &status, now()).unwrap();
    let text = String::from_utf8(output).unwrap();
    assert!(
        text.starts_with("meshway-lax  100.64.0.4  retrying\n"),
        "{text}"
    );
    assert!(
        text.contains("  Data plane failed · mode tun · changed 3m 0s ago\n"),
        "{text}"
    );
    assert!(
        text.contains("  Problem    dns\n             DNS observe_deadline check failed\n"),
        "{text}"
    );
}
