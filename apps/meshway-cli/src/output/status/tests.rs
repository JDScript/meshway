use super::*;
use serde_json::json;

const NYC: &str = "00000000-0000-4000-8000-000000000002";
const SHA: &str = "00000000-0000-4000-8000-000000000003";
const OLD: &str = "00000000-0000-4000-8000-000000000004";
const PLA: &str = "00000000-0000-4000-8000-000000000005";

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
    let reference = json!({"revision":42,"serial":17,"digest":"01".repeat(32)});
    serde_json::from_value(json!({
        "processId":1,"revision":3,"desiredUp":true,"phase":"running","server":"http://127.0.0.1:8420",
        "observedAt":"2026-10-11T12:00:00Z","enrollmentRequestId":null,"enrollmentUrl":null,
        "identity":null,"authenticated":true,"sync":"connected","lastSyncAt":"2026-10-11T11:59:48Z",
        "syncError":null,"instance":null,
        "netmap":{"netmap":reference,"issuedAt":"2026-10-11T11:00:00Z","expiresAt":"2026-10-12T11:59:00Z","missingArtifacts":[],"artifactProblem":null,
            "capture":{"managedIpv4":4341,"sinkIpv6":2256,"routes":12680,"unavailable":null},
            "routing":{"requestedProfile":"work","version":2,"appliedProfile":"work","requestedUnavailable":null,
                "defaultProfile":"mesh","locked":false,"profiles":[],"auto":null}},
        "saved":reference,"applied":reference,"dataPlane":"ready","networkMode":"tun",
        "node":{"hostname":"meshway-lax","addresses":{"ipv4":"100.64.0.4","ipv6":null}},
        "peers":[nyc, sha, peer(OLD, "meshway-old", "100.64.0.9", "offline"), planck],
        "sockets":[
            {"resource":{"kind":"listener","key":"native"},"boundAddress":"0.0.0.0:35000",
             "receiveBuffer":14680064,"receiveRequested":7340032,"sendBuffer":4194304,"sendRequested":2097152},
            {"resource":{"kind":"listener","key":"native6"},"boundAddress":"[::]:35000",
             "receiveBuffer":425984,"receiveRequested":7340032,"sendBuffer":4194304,"sendRequested":2097152}
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
    assert!(text.contains("  Data plane ready · mode tun\n"));
    let header = lines
        .iter()
        .position(|line| line.starts_with("PEER"))
        .unwrap();
    assert_eq!(
        lines[header],
        "PEER         MESH IPV4   STATE         TRANSPORT   ENDPOINT            MTU        RTT"
    );
    assert_eq!(
        lines[header + 1],
        "meshway-nyc  100.64.0.6  relay         native_udp  via meshway-sha                0.31 ms"
    );
    assert_eq!(
        lines[header + 2],
        "meshway-sha  100.64.0.5  direct        native_udp  192.168.97.3:35000  1444/1330  0.14 ms"
    );
    // No measurement is not zero.
    assert_eq!(lines[header + 3], "meshway-old  100.64.0.9  offline");
    assert_eq!(
        lines[header + 4],
        "meshway-pla  100.64.0.7  relay (idle)  native_udp  via meshway-sha"
    );
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
            "14.0",
            "MiB",
            "4.0",
            "MiB"
        ]
    );
    assert!(lines[header + 2].contains("416 KiB (requested 7.0 MiB, limited)"));
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
    assert_eq!(value["peers"].as_array().unwrap().len(), 4);
    assert_eq!(value["peers"][3]["state"], "relayIdle");
    assert_eq!(value["node"]["hostname"], "meshway-lax");
    assert!(value["peers"][2]["rtt"].is_null() && value["identity"].is_null());
    let decoded: Status = serde_json::from_value(value).unwrap();
    assert_eq!(decoded.peers, status().peers);
}
