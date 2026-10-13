use super::*;
use meshway_model::state::{AdjacencyStatus, LinkPhase, LinkState};

/// The setup peer as a second reporting Node with its own admission.
fn peer_side(
    admission: &ReportAdmission,
    report: &StateReport,
    scope: &mut VisibilityScope,
) -> (ReportAdmission, StateReport) {
    let peer = id(2);
    let mut other = admission.clone();
    other.node_id = peer;
    other.instance_id = id(6);
    for context in &mut other.contexts {
        context.peers = BTreeSet::from([admission.node_id]);
        context.neighbor_transports =
            BTreeMap::from([(admission.node_id, BTreeSet::from(["native_udp".into()]))]);
    }
    let mut peer_report = report.clone();
    peer_report.instance_id = other.instance_id;
    let mut current = scope.current[&admission.node_id].clone();
    current.instance_id = Some(other.instance_id);
    scope.current.insert(peer, current);
    (other, peer_report)
}

fn link(peer: NodeId, initiator: NodeId, now: DateTime<Utc>) -> (NodeId, Vec<LinkState>) {
    (
        peer,
        vec![LinkState {
            acquisition: None,
            link_id: "l1".into(),
            socket_id: "s1".into(),
            transport: "native_udp".into(),
            initiator_node_id: initiator,
            phase: LinkPhase::Authenticated,
            business_dependencies: 1,
            observation_ids: vec![],
            observed_at: now,
            expires_at: now + Duration::seconds(30),
            since: None,
        }],
    )
}

fn status(view: &StateView) -> Vec<(NodeId, NodeId, AdjacencyConnection)> {
    view.adjacencies
        .iter()
        .map(|a| (a.a, a.b, a.status))
        .collect()
}

#[test]
fn member_and_owner_see_the_same_conclusion_when_only_the_hidden_side_reports_a_channel() {
    let (admission, mut report, mut scope, now) = setup();
    let (node, peer) = (admission.node_id, id(2));
    let (other, mut peer_report) = peer_side(&admission, &report, &mut scope);
    // The member's own Node reports nothing about the edge; only the peer,
    // whose report the member reads as a peer projection, has the channel.
    report.state.adjacencies = BTreeMap::from([(peer, AdjacencyStatus::Idle)]);
    peer_report.state.links = BTreeMap::from([link(node, peer, now)]);
    let mut hub = StateHub::new().unwrap();
    hub.accept_report(&admission, report, now).unwrap();
    hub.accept_report(&other, peer_report, now).unwrap();
    let member = hub.query(&scope, None, now).unwrap();
    assert!(member.links.is_empty());
    assert_eq!(
        status(&member),
        [(node, peer, AdjacencyConnection::Connected)]
    );
    scope.full.insert(peer);
    let owner = hub.query(&scope, None, now).unwrap();
    assert_eq!(owner.links.len(), 1);
    assert_eq!(owner.links[0].evidence, LinkEvidence::OneSided);
    assert_eq!(owner.adjacencies, member.adjacencies);
    // A single-Node read keeps the same pair conclusion.
    let single = hub.query(&scope, Some(&[node]), now).unwrap();
    assert_eq!(single.adjacencies, member.adjacencies);
    // Pairs with no fully visible endpoint are not shown.
    scope.full.clear();
    assert!(hub.query(&scope, None, now).unwrap().adjacencies.is_empty());
}

#[test]
fn channels_win_then_unreachable_then_idle_and_stale_or_conflicting_is_unknown() {
    let (admission, mut report, mut scope, now) = setup();
    let (node, peer) = (admission.node_id, id(2));
    let (other, mut peer_report) = peer_side(&admission, &report, &mut scope);
    report.state.adjacencies = BTreeMap::from([(peer, AdjacencyStatus::Idle)]);
    peer_report.state.adjacencies = BTreeMap::from([(node, AdjacencyStatus::Unreachable)]);
    let mut hub = StateHub::new().unwrap();
    hub.accept_report(&admission, report.clone(), now).unwrap();
    hub.accept_report(&other, peer_report.clone(), now).unwrap();
    scope.full.insert(peer);
    assert_eq!(
        status(&hub.query(&scope, None, now).unwrap()),
        [(node, peer, AdjacencyConnection::Unreachable)]
    );
    // Both endpoints report the same channel differently.
    report.sequence = Sequence::new(2).unwrap();
    report.state.adjacencies.clear();
    report.state.links = BTreeMap::from([link(peer, node, now)]);
    peer_report.sequence = Sequence::new(2).unwrap();
    peer_report.state.adjacencies.clear();
    peer_report.state.links = BTreeMap::from([link(node, peer, now)]);
    hub.accept_report(&admission, report.clone(), now).unwrap();
    hub.accept_report(&other, peer_report, now).unwrap();
    let view = hub.query(&scope, None, now).unwrap();
    assert_eq!(view.links[0].evidence, LinkEvidence::Conflicting);
    assert_eq!(status(&view), [(node, peer, AdjacencyConnection::Unknown)]);
    // A channel survives only while its reports are current.
    let later = hub
        .query(&scope, None, now + Duration::seconds(61))
        .unwrap();
    assert_eq!(status(&later), [(node, peer, AdjacencyConnection::Unknown)]);
    // A closed channel is no connection.
    report.sequence = Sequence::new(3).unwrap();
    report.state.links.get_mut(&peer).unwrap()[0].phase = LinkPhase::Closed;
    hub.accept_report(&admission, report, now).unwrap();
    scope.current.remove(&peer);
    assert_eq!(
        status(&hub.query(&scope, None, now).unwrap()),
        [(node, peer, AdjacencyConnection::Unknown)]
    );
}

/// A report accepted while the peer was visible keeps its adjacency entry;
/// after the reader's permission shrinks, the full report read omits it, as
/// it omits links.
#[test]
fn a_full_report_omits_adjacencies_of_peers_no_longer_visible() {
    let (admission, mut report, mut scope, now) = setup();
    let peer = id(2);
    report.state.adjacencies = BTreeMap::from([(peer, AdjacencyStatus::Unreachable)]);
    let mut hub = StateHub::new().unwrap();
    hub.accept_report(&admission, report, now).unwrap();
    let full = |view: &StateView| match &view.nodes[&admission.node_id].report {
        Some(VisibleReport::Full { report }) => (*report).clone(),
        other => panic!("{other:?}"),
    };
    let before = full(&hub.query(&scope, None, now).unwrap());
    assert!(before.adjacencies.contains_key(&peer));
    scope.visible.remove(&peer);
    let after = full(&hub.query(&scope, None, now).unwrap());
    assert!(after.adjacencies.is_empty(), "{:?}", after.adjacencies);
}
