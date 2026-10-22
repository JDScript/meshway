use super::*;
use rtnetlink::packet_route::address::AddressMessage;

/// F6a B8: an empty or buffer-filling (truncated) datagram proves
/// nothing: the monitor ends ownership as unavailable, re-proved later.
/// Only a notification decoded and found foreign is a conflict.
#[test]
fn empty_or_truncated_datagrams_end_ownership_as_unavailable() {
    let unavailable = crate::OwnershipEnd::Unavailable {
        check: "monitor_unavailable",
    };
    for size in [0, 65536, 70000] {
        let error = complete(size, 65536).unwrap_err();
        assert_eq!(ended(&error), unavailable, "size {size}");
    }
    complete(16, 65536).unwrap();
    assert_eq!(
        ended(&Error::Conflict),
        crate::OwnershipEnd::Conflict { check: "monitor" }
    );
}

/// RC-1 M4 (G6): the monitor's own failure (its state lock, a stopped
/// worker) reads nothing back: an unavailable end, never a conflict.
#[test]
fn a_monitor_internal_failure_ends_ownership_as_unavailable() {
    assert_eq!(
        ended(&Error::Worker),
        crate::OwnershipEnd::Unavailable {
            check: "monitor_unavailable"
        }
    );
}

#[test]
fn first_conflict_is_bounded_and_omits_unselected_attributes_and_expected_bytes() {
    let mut address = AddressMessage::default();
    address.header.index = 9;
    address.header.prefix_len = 64;
    address
        .attributes
        .push(AddressAttribute::Address("fe80::1234".parse().unwrap()));
    address
        .attributes
        .push(AddressAttribute::Label("unselected-private-label".into()));
    let facts = Facts::from_payload(&NetlinkPayload::InnerMessage(
        RouteNetlinkMessage::NewAddress(address),
    ));
    let mut expected = Expected {
        tun: Some((
            9,
            "unselected-tun-name".into(),
            1280,
            vec!["100.96.0.1".parse().unwrap()],
        )),
        requested_drain: 4,
        drained: 3,
        ..Default::default()
    };
    expected.events.push_back(Event {
        rule: false,
        deleted: false,
        bytes: b"unselected-raw-message".to_vec(),
    });
    facts.reject(&mut expected, "tun_address_changed");
    let first = expected.first_conflict.clone().unwrap();
    assert!(first.len() < 1024);
    assert!(first.contains("RTM_NEWADDR") && first.contains("fe80::1234"));
    assert!(!first.contains("unselected-"));
    facts.reject(&mut expected, "later_conflict");
    assert_eq!(expected.first_conflict.as_deref(), Some(first.as_str()));
}

#[test]
fn owed_capture_table_events_are_consumed_once_each() {
    let mut expected = Expected::default();
    expected.expect_egress(false, vec![1]);
    expected.expect_egress(false, vec![1]);
    expected.expect_egress(true, vec![1]);
    assert!(!expected.egress_event(true, vec![2]));
    assert!(expected.egress_event(false, vec![1]));
    assert!(expected.egress_event(false, vec![1]));
    assert!(!expected.egress_event(false, vec![1]));
    assert!(expected.egress_event(true, vec![1]));
    assert!(expected.egress_events.is_empty());
}

#[test]
fn a_dns_anchor_being_created_is_not_an_external_network_change() {
    use rtnetlink::packet_route::link::LinkMessage;
    let state = Expected {
        anchor_creating: Some(("mwd0123456789".into(), None)),
        ..Default::default()
    };
    let link = |index, name: &str| {
        let mut link = LinkMessage::default();
        link.header.index = index;
        link.header.flags = LinkFlags::Up;
        link.attributes.push(LinkAttribute::IfName(name.into()));
        NetlinkPayload::InnerMessage(RouteNetlinkMessage::NewLink(link))
    };
    let address = |index| {
        let mut address = AddressMessage::default();
        address.header.index = index;
        NetlinkPayload::InnerMessage(RouteNetlinkMessage::NewAddress(address))
    };
    assert!(!network_changed(&state, &link(7, "mwd0123456789"), 0));
    assert!(network_changed(&state, &address(7), 0));
    let state = Expected {
        anchor_creating: Some(("mwd0123456789".into(), Some(7))),
        ..Default::default()
    };
    assert!(!network_changed(&state, &address(7), 0));
    assert!(!network_changed(&state, &link(7, "mwd0123456789"), 0));
    assert!(network_changed(&state, &link(8, "eth1"), 0));
    assert!(network_changed(&state, &address(8), 0));
}

#[test]
fn a_link_that_is_not_up_is_not_a_network_change() {
    use rtnetlink::packet_route::link::LinkMessage;
    let mut idle = LinkMessage::default();
    idle.header.index = 9;
    idle.attributes.push(LinkAttribute::IfName("ppp0".into()));
    let state = Expected::default();
    for message in [
        RouteNetlinkMessage::NewLink(idle.clone()),
        RouteNetlinkMessage::DelLink(idle.clone()),
    ] {
        assert!(!network_changed(
            &state,
            &NetlinkPayload::InnerMessage(message),
            0
        ));
    }
    idle.header.flags = LinkFlags::Up;
    assert!(network_changed(
        &state,
        &NetlinkPayload::InnerMessage(RouteNetlinkMessage::DelLink(idle)),
        0
    ));
}
