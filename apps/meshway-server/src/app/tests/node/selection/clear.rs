//! Clearing a persistent selection through both endpoints: the same
//! commit, its compare-and-swap base, the activity and the default it leaves.
use super::*;

#[tokio::test]
async fn a_node_clears_its_selection_under_its_version_and_the_default_applies() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let lab = selection().await;
    // Nothing to clear: no change, no activity, no new netmap.
    let before = lab.netmap(lab.a).await;
    let nothing = lab.node_clear(None).await.unwrap().selection;
    assert_eq!((nothing.requested_profile, nothing.version), (None, None));
    assert_eq!(
        lab.netmap(lab.a).await.reference().serial,
        before.reference().serial
    );
    assert!(lab.activities().await.is_empty());

    lab.node_select(None, "work").await.unwrap();
    // The compare-and-swap base is the held version, as for a selection.
    assert_eq!(
        lab.node_clear(None).await.unwrap_err(),
        Error::VersionConflict
    );
    let selected = lab.netmap(lab.a).await;
    let cleared = lab.node_clear(Some(1)).await.unwrap().selection;
    assert_eq!(
        (
            cleared.requested_profile.as_deref(),
            cleared.version,
            cleared.requested_unavailable,
            cleared.applied_profile.as_str()
        ),
        (None, Version::new(2).ok(), None, "mesh")
    );
    // The new netmap carries the cleared selection's version, the base of
    // the next change, and the same state.
    let after = lab.netmap(lab.a).await;
    assert!(after.reference().serial > selected.reference().serial);
    assert_eq!(
        after.netmap.projection.selection_versions.get(&lab.a),
        Version::new(2).ok().as_ref()
    );
    assert!(
        !after
            .netmap
            .projection
            .selected_profiles
            .contains_key(&lab.a)
    );
    assert_eq!(
        meshway_policy::compile::netmap::routing_selection(&after.netmap).unwrap(),
        cleared
    );
    // Versions keep growing: the clearing used 2, the next change's base.
    let again = lab.node_select(Some(2), "auto").await.unwrap().selection;
    assert_eq!(again.version.map(Version::get), Some(3));
    let events = lab.activities().await;
    assert_eq!(events.len(), 3);
    assert!(matches!(event_profile(&events[1]), (node, None) if node == lab.a));
    assert!(matches!(events[1].actor, ActivityActor::Node { node_id } if node_id == lab.a));
    assert_eq!(event_profile(&events[0]).1.as_deref(), Some("auto"));
}

fn event_profile(event: &meshway_protocol::history::ActivityEvent) -> (NodeId, Option<String>) {
    match &event.target {
        ActivityTarget::RoutingSelection { node_id, profile } => (*node_id, profile.clone()),
        _ => panic!("routing.select without a selection target"),
    }
}

#[tokio::test]
async fn clearing_a_selection_the_assignment_no_longer_allows_restores_the_default_auto() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let lab = selection().await;
    let (a, m, b) = (lab.a, lab.m, lab.b);
    let with_default = |allowed: &[&str], default: &str| ProfileAssignment {
        target: vec![format!("node:{a}")],
        allowed: allowed.iter().map(|p| (*p).to_owned()).collect(),
        default: default.into(),
        locked: false,
    };
    lab.node_select(None, "work").await.unwrap();
    // The administrator removes `work` and makes `auto` the default: the
    // persistent selection stays, not applied.
    let global = format!("global:{b}");
    let narrowed = with_default(&["auto", "mesh", &global], "auto");
    configure(&lab.app, &lab.owner, lab.network, move |config| {
        config.routing.egress.assignments = vec![narrowed, assignment(m, &["mesh", "work"], false)];
        config.nodes.get_mut(&a).unwrap().region = Some("US".into());
        config.routing.egress.auto = Input::Value(AutoProfile {
            rules: vec![CountryProfile {
                countries: vec!["US".into()],
                profile: format!("global:{b}"),
            }],
            fallback: "mesh".into(),
        });
    })
    .await;
    let stuck =
        meshway_policy::compile::netmap::routing_selection(&lab.netmap(a).await.netmap).unwrap();
    assert_eq!(stuck.requested_profile.as_deref(), Some("work"));
    assert_eq!(
        stuck.requested_unavailable,
        Some(SelectionUnavailable::NotAllowed)
    );
    assert_eq!(stuck.applied_profile, "auto");
    // The owner clears it: the selection is gone and `auto` resolves as before.
    let request = rid();
    let cleared = lab.user_clear(&lab.owner, a, request).await.unwrap();
    assert_eq!(
        (
            cleared.requested_profile.as_deref(),
            cleared.requested_unavailable,
            cleared.applied_profile.as_str()
        ),
        (None, None, "auto")
    );
    let carried =
        meshway_policy::compile::netmap::routing_selection(&lab.netmap(a).await.netmap).unwrap();
    assert_eq!(carried, cleared);
    let auto = carried.auto.unwrap();
    assert_eq!(
        (auto.profile, auto.basis),
        (format!("global:{b}"), AutoBasis::Label)
    );
    // A retry is the same clearing; its ID cannot select a profile.
    let events = lab.activities().await.len();
    assert_eq!(
        lab.user_clear(&lab.owner, a, request).await.unwrap(),
        cleared
    );
    assert_eq!(lab.activities().await.len(), events);
    assert_eq!(
        lab.app
            .execution()
            .select_user_profile(
                &lab.owner,
                lab.network,
                a,
                SelectNodeRoutingProfileRequest {
                    request_id: request,
                    profile: "mesh".into(),
                },
            )
            .await
            .unwrap_err(),
        Error::RequestConflict
    );
}

#[tokio::test]
async fn members_clear_only_their_own_devices_and_a_lock_does_not_refuse_it() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let lab = selection().await;
    let execution = lab.app.execution();
    execution
        .select_user_profile(
            &lab.owner,
            lab.network,
            lab.a,
            SelectNodeRoutingProfileRequest {
                request_id: rid(),
                profile: "work".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        lab.user_clear(&lab.member, lab.a, rid()).await.unwrap_err(),
        Error::NotFound
    );
    execution
        .select_user_profile(
            &lab.member,
            lab.network,
            lab.m,
            SelectNodeRoutingProfileRequest {
                request_id: rid(),
                profile: "work".into(),
            },
        )
        .await
        .unwrap();
    let own = lab.user_clear(&lab.member, lab.m, rid()).await.unwrap();
    assert_eq!(
        (own.requested_profile, own.applied_profile.as_str()),
        (None, "mesh")
    );
    // A lock pins the default, which is what clearing leaves.
    let (a, m) = (lab.a, lab.m);
    let global = format!("global:{}", lab.b);
    configure(&lab.app, &lab.owner, lab.network, move |config| {
        config.routing.egress.assignments = vec![
            assignment(a, &["mesh", "work", "auto", &global], true),
            assignment(m, &["mesh", "work"], false),
        ];
    })
    .await;
    let locked = lab.user_clear(&lab.owner, lab.a, rid()).await.unwrap();
    assert_eq!(
        (
            locked.requested_profile,
            locked.locked,
            locked.applied_profile.as_str()
        ),
        (None, true, "mesh")
    );
    let events = lab.activities().await;
    let member = &events[1];
    assert!(matches!(&member.actor, ActivityActor::User { .. }));
    assert_eq!(event_profile(member), (lab.m, None));
    assert_eq!(event_profile(&events[0]), (lab.a, None));
}

/// F6a A1: a cleared selection keeps its version as the compare-and-swap
/// base. A change made before the clearing (an empty base, or the selected
/// version) is stale afterwards: no A-B-A through the cleared state.
#[tokio::test]
async fn a_cleared_selection_keeps_its_version_and_a_stale_base_conflicts() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let lab = selection().await;
    lab.node_select(None, "work").await.unwrap();
    let cleared = lab.node_clear(Some(1)).await.unwrap().selection;
    assert_eq!(cleared.version.map(Version::get), Some(2));
    // Read before the first selection, or before the clearing: both stale.
    assert_eq!(
        lab.node_select(None, "auto").await.unwrap_err(),
        Error::VersionConflict
    );
    assert_eq!(
        lab.node_select(Some(1), "auto").await.unwrap_err(),
        Error::VersionConflict
    );
    // Clearing again under the cleared version changes nothing.
    let serial = lab.netmap(lab.a).await.reference().serial;
    let again = lab.node_clear(Some(2)).await.unwrap().selection;
    assert_eq!(again, cleared);
    assert_eq!(lab.netmap(lab.a).await.reference().serial, serial);
    assert_eq!(lab.activities().await.len(), 2);
    // The current base selects.
    let selected = lab.node_select(Some(2), "auto").await.unwrap().selection;
    assert_eq!(selected.version.map(Version::get), Some(3));
    // The user view and the inventory agree with the Node's netmap.
    let carried =
        meshway_policy::compile::netmap::routing_selection(&lab.netmap(lab.a).await.netmap)
            .unwrap();
    assert_eq!(carried, selected);
    assert_eq!(
        lab.app
            .execution()
            .routing_selection(&lab.owner, lab.network, lab.a)
            .await
            .unwrap(),
        selected
    );
}

/// F6a A2: a user's clearing without a selection still leaves its receipt:
/// its retry after a later selection returns that selection and clears
/// nothing, and its request ID cannot select a profile.
#[tokio::test]
async fn a_retried_clearing_without_a_selection_never_clears_a_later_one() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let lab = selection().await;
    let request = rid();
    let nothing = lab.user_clear(&lab.owner, lab.a, request).await.unwrap();
    assert_eq!((nothing.requested_profile, nothing.version), (None, None));
    let events = lab.activities().await;
    assert_eq!(events.len(), 1);
    assert_eq!(event_profile(&events[0]), (lab.a, None));
    let serial = lab.netmap(lab.a).await.reference().serial;
    let selected = lab
        .app
        .execution()
        .select_user_profile(
            &lab.owner,
            lab.network,
            lab.a,
            SelectNodeRoutingProfileRequest {
                request_id: rid(),
                profile: "work".into(),
            },
        )
        .await
        .unwrap();
    assert!(lab.netmap(lab.a).await.reference().serial > serial);
    // The retry is the same clearing: it finds its receipt.
    assert_eq!(
        lab.user_clear(&lab.owner, lab.a, request).await.unwrap(),
        selected
    );
    assert_eq!(lab.activities().await.len(), 2);
    // Its ID on another Node, or for a selection, is a conflict.
    assert_eq!(
        lab.user_clear(&lab.owner, lab.m, request)
            .await
            .unwrap_err(),
        Error::RequestConflict
    );
    assert_eq!(
        lab.app
            .execution()
            .select_user_profile(
                &lab.owner,
                lab.network,
                lab.a,
                SelectNodeRoutingProfileRequest {
                    request_id: request,
                    profile: "work".into(),
                },
            )
            .await
            .unwrap_err(),
        Error::RequestConflict
    );
}

/// S4 (OPS16): a selection cleared under alpha.16, whose stored netmap an
/// earlier build compiled without the cleared version. The Server start by
/// this build compiles it again: the Node's netmap carries the version its
/// next change is based on, without any configuration or selection change.
#[tokio::test]
async fn a_start_by_another_build_restores_the_cleared_version_in_the_netmap() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let lab = selection().await;
    lab.node_select(None, "work").await.unwrap();
    lab.node_clear(Some(1)).await.unwrap();
    let current = lab.netmap(lab.a).await;
    assert_eq!(
        current.netmap.projection.selection_versions.get(&lab.a),
        Version::new(2).ok().as_ref()
    );
    let mut earlier = (*current.netmap).clone();
    earlier.serial = earlier.serial.checked_next().unwrap();
    earlier.projection.selection_versions.remove(&lab.a);
    let earlier = meshway_configuration::netmap::CompiledNetmap {
        digest: earlier.content_digest().unwrap(),
        netmap: std::sync::Arc::new(earlier),
    };
    let held = earlier.reference();
    let (sealer, network, a) = (lab.app.sealer(), lab.network, lab.a);
    lab.app
        .executor
        .handle()
        .write(move |tx| {
            let now = crate::store::execution::now()?;
            crate::store::netmap::store_as_another_build(tx, &sealer, network, a, &earlier, now)
        })
        .await
        .unwrap();
    assert!(
        !lab.netmap(a)
            .await
            .netmap
            .projection
            .selection_versions
            .contains_key(&a)
    );
    let Selection { _dir: dir, app, .. } = lab;
    app.shutdown().await.unwrap();
    let app = App::open(crate::settings::Settings::new(
        dir.path().canonicalize().unwrap().join("data"),
    ))
    .await
    .unwrap();
    let netmaps = app.netmaps.clone();
    let refreshed = app
        .executor
        .handle()
        .read(move |tx| netmaps.current(tx, network, a))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(refreshed.netmap.serial.get(), held.serial.get() + 1);
    assert_eq!(refreshed.digest, current.digest);
    assert_eq!(
        refreshed.netmap.projection.selection_versions.get(&a),
        Version::new(2).ok().as_ref()
    );
    app.shutdown().await.unwrap();
}
