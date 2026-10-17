//! `nodes.<id>.exitFamilies`: IPv6 is carried in addition to IPv4, each
//! family once, stored ascending; the default `["ipv4"]` is omitted.
use super::*;
use meshway_model::exit::ExitFamily;

#[test]
fn exit_families_must_include_ipv4_and_name_each_family_once() {
    for families in [
        vec![],
        vec![ExitFamily::Ipv6],
        vec![ExitFamily::Ipv4, ExitFamily::Ipv4],
    ] {
        let mut c = with_nodes();
        c.nodes.get_mut(&id(2)).unwrap().exit_families = families.clone();
        let found = issues(normalize_with_dependencies(c, &context()));
        assert!(
            found.iter().any(|i| i.code == IssueCode::InvalidExitFamily
                && i.path == format!("/nodes/{}/exitFamilies", id(2))),
            "{families:?}: {found:?}"
        );
    }
    let mut c = with_nodes();
    c.nodes.get_mut(&id(2)).unwrap().exit_families = vec![ExitFamily::Ipv6, ExitFamily::Ipv4];
    let saved = normalize_with_dependencies(c, &context()).unwrap();
    assert_eq!(
        saved.configuration.nodes[&id(2)].exit_families,
        [ExitFamily::Ipv4, ExitFamily::Ipv6]
    );
    let text = serde_json::to_value(&saved.configuration).unwrap();
    assert!(
        text["nodes"][id(1).to_string()]
            .get("exitFamilies")
            .is_none()
    );
    assert_eq!(
        text["nodes"][id(2).to_string()]["exitFamilies"],
        serde_json::json!(["ipv4", "ipv6"])
    );
}
