use super::*;
use serde_json::json;

const PARIS: &str = "00000000-0000-4000-8000-000000000002";

fn peers() -> Vec<PeerStatus> {
    serde_json::from_value(json!([{
        "nodeId":PARIS,"hostname":"paris","addresses":{"ipv4":"100.64.0.2","ipv6":null},
        "state":"idle","transport":null,"endpoint":null,"via":[],"rtt":null
    }]))
    .unwrap()
}

fn selection(value: serde_json::Value) -> RoutingSelectionState {
    let mut base = json!({
        "requestedProfile":"work","version":3,"appliedProfile":"work","requestedUnavailable":null,
        "defaultProfile":"mesh","locked":false,"auto":null,
        "profiles":[
            {"name":"mesh","kind":"mesh","rules":0,"exits":[],"fallback":{"type":"unmanaged"}},
            {"name":"work","kind":"custom","rules":2,"exits":[PARIS],
             "fallback":{"type":"exit","nodes":[format!("node:{PARIS}")],"via":[]}},
            {"name":format!("global:{PARIS}"),"kind":"global","rules":0,"exits":[PARIS],
             "fallback":{"type":"exit","nodes":[format!("node:{PARIS}")],"via":[]}},
            {"name":"auto","kind":"auto","rules":0,"exits":[],"fallback":null}
        ]
    });
    for (key, field) in value.as_object().unwrap() {
        base[key] = field.clone();
    }
    serde_json::from_value(base).unwrap()
}

fn render(term: &Term, selection: &RoutingSelectionState) -> String {
    let mut output = Vec::new();
    let peers = peers();
    write(&mut output, term, selection, &Names(&peers)).unwrap();
    String::from_utf8(output).unwrap()
}

#[test]
fn plain_text_names_exits_and_marks_applied_requested_and_default() {
    let text = render(&Term::plain(), &selection(json!({})));
    assert!(!text.contains('\u{1b}') && !text.contains('│'));
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], "Exit profile work");
    assert_eq!(lines[1], "  Requested  work · version 3");
    assert_eq!(lines[2], "  Default    mesh · not locked");
    assert_eq!(lines[3], "");
    assert_eq!(
        &lines[4..],
        [
            "PROFILE                                      KIND    STATE    SUMMARY",
            "mesh                                         mesh    default  Mesh only; other external traffic unmanaged",
            "work                                         custom  applied  2 rules · exits paris · otherwise exit paris",
            "global:00000000-0000-4000-8000-000000000002  global           all external traffic: exit paris",
            "auto                                         auto             the allowed profile the Node's region selects",
        ]
    );
}

#[test]
fn an_unavailable_request_and_auto_resolution_are_explained() {
    let locked = render(
        &Term::plain(),
        &selection(json!({"appliedProfile":"mesh","requestedUnavailable":"locked","locked":true})),
    );
    assert!(locked.starts_with("Exit profile mesh (locked)\n"));
    assert!(locked.contains("  Requested  work · not applied (locked to the default)\n"));
    assert!(locked.contains("  Default    mesh · locked by an administrator\n"));
    assert!(locked.contains("\nmesh  "));
    assert!(locked.contains("applied, default"));
    let auto = render(
        &Term::plain(),
        &selection(json!({"requestedProfile":"auto","appliedProfile":"auto",
            "auto":{"profile":"mesh","basis":"noTrustedRegion"}})),
    );
    assert!(auto.contains("  Auto       → mesh (no trusted region yet; configured fallback)\n"));
    let none = render(
        &Term::plain(),
        &selection(json!({"requestedProfile":null,"version":null,"appliedProfile":"mesh"})),
    );
    assert!(none.contains("  Requested  none (the default applies)\n"));
}

#[test]
fn a_terminal_gets_colors_and_a_box_table() {
    let text = render(&Term::rich(), &selection(json!({})));
    assert!(text.contains('\u{1b}') && text.contains('╭') && text.contains('│'));
}
