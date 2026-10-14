//! Routing profile selection round trip against the real Server: Control
//! sends the held netmap's version, the Server commits and recompiles, and
//! the next sync carries the selection Control reports.
use super::support::{Owner, Server, request};
use meshway_configuration::NetworkConfigurationInput;
use meshway_configuration::selection::SelectionUnavailable;
use meshway_control::{Control, Error};
use meshway_control_client::ClientError;
use meshway_protocol::{
    ConfigurationRequest, Problem, SelectNodeRoutingProfileRequest, WireSecret,
};
use meshway_test_server::client::{
    ApplyConfigurationPath, GetConfigurationSourcePath, GetConfigurationSourceQuery,
    SetNodeRoutingSelectionPath,
};
use std::collections::BTreeMap;
use tokio::sync::Mutex;

async fn configure(server: &Server, owner: &Owner) {
    let source = server
        .client
        .get_configuration_source(
            &GetConfigurationSourcePath {
                network_id: owner.network,
            },
            &GetConfigurationSourceQuery::default(),
            owner.token.expose(),
        )
        .await
        .unwrap();
    let mut input: NetworkConfigurationInput = source.configuration.clone();
    input.routing.egress = serde_json::from_value(serde_json::json!({
        "profiles": {"local": {"fallback": {"type": "direct"}}},
        "assignments": [{"target": ["*"], "allowed": ["mesh", "local"], "default": "mesh"}]
    }))
    .unwrap();
    server
        .client
        .apply_configuration(
            &ApplyConfigurationPath {
                network_id: owner.network,
            },
            &ConfigurationRequest::Document {
                expected_revision: source.revision,
                request_id: request(),
                source_jsonc: WireSecret::new(serde_json::to_string(&input).unwrap()),
            },
            owner.token.expose(),
        )
        .await
        .unwrap();
}

fn problem(error: &Error) -> Option<&Problem> {
    match error {
        Error::Server(ClientError::Http {
            problem: Some(body),
            ..
        }) => Some(&body.problem),
        _ => None,
    }
}

#[tokio::test]
async fn a_selection_reaches_the_next_netmap_and_refusals_keep_the_session() {
    let server = Server::start().await;
    let owner = server.owner().await;
    let mut control = server
        .control(&owner, &server.origin, "selection", BTreeMap::new())
        .await;
    configure(&server, &owner).await;
    crate::support::sync(&mut control).await.unwrap();
    let initial = control.status().routing.unwrap();
    assert_eq!(
        (
            initial.requested_profile,
            initial.version,
            initial.applied_profile.as_str()
        ),
        (None, None, "mesh")
    );
    let node = control.status().binding.unwrap().node_id;
    let held = control.status().held.unwrap();
    let control = Mutex::new(control);

    let committed = meshway_control::shared::select_profile(&control, "local".into())
        .await
        .unwrap();
    assert_eq!(committed.applied_profile, "local");
    assert_eq!(committed.version.map(|v| v.get()), Some(1));
    // Control keeps nothing of the answer: the held netmap still decides.
    assert_eq!(
        control
            .lock()
            .await
            .status()
            .routing
            .unwrap()
            .applied_profile,
        "mesh"
    );
    crate::support::sync(&mut *control.lock().await)
        .await
        .unwrap();
    let status = control.lock().await.status();
    assert!(status.held.unwrap().serial > held.serial);
    assert_eq!(status.routing.unwrap(), committed);

    // A profile outside the allowed set is refused with them; the session
    // and instance stay.
    let refused = meshway_control::shared::select_profile(&control, "global:x".into())
        .await
        .unwrap_err();
    match problem(&refused) {
        Some(Problem::RoutingProfileUnavailable { params, .. }) => {
            assert_eq!(params.reason, SelectionUnavailable::NotAllowed);
            assert_eq!(params.allowed, ["mesh", "local"]);
        }
        other => panic!("{other:?}"),
    }
    let status = control.lock().await.status();
    assert!(status.authenticated && status.instance.is_some());

    // A user changed the selection meanwhile: the held version is stale
    // until the next netmap, and the session stays.
    server
        .client
        .set_node_routing_selection(
            &SetNodeRoutingSelectionPath {
                network_id: owner.network,
                node_id: node,
            },
            &SelectNodeRoutingProfileRequest {
                request_id: request(),
                profile: "mesh".into(),
            },
            owner.token.expose(),
        )
        .await
        .unwrap();
    let stale = meshway_control::shared::select_profile(&control, "local".into())
        .await
        .unwrap_err();
    assert!(matches!(
        problem(&stale),
        Some(Problem::VersionConflict { .. })
    ));
    assert!(control.lock().await.status().instance.is_some());
    crate::support::sync(&mut *control.lock().await)
        .await
        .unwrap();
    assert_eq!(
        control
            .lock()
            .await
            .status()
            .routing
            .unwrap()
            .version
            .map(|v| v.get()),
        Some(2)
    );
    let again = meshway_control::shared::select_profile(&control, "local".into())
        .await
        .unwrap();
    assert_eq!(again.version.map(|v| v.get()), Some(3));

    // Clearing under a stale base is refused like a selection.
    let stale = meshway_control::shared::clear_profile(&control)
        .await
        .unwrap_err();
    assert!(matches!(
        problem(&stale),
        Some(Problem::VersionConflict { .. })
    ));
    crate::support::sync(&mut *control.lock().await)
        .await
        .unwrap();
    // Clearing leaves no selection: the default applies with the next netmap.
    let cleared = meshway_control::shared::clear_profile(&control)
        .await
        .unwrap();
    assert_eq!(
        (
            cleared.requested_profile.as_deref(),
            cleared.version,
            cleared.applied_profile.as_str()
        ),
        (None, None, "mesh")
    );
    crate::support::sync(&mut *control.lock().await)
        .await
        .unwrap();
    assert_eq!(control.lock().await.status().routing.unwrap(), cleared);
    // Clearing nothing changes nothing; the next selection's version is
    // still above every earlier one (the clearing used 4).
    let nothing = meshway_control::shared::clear_profile(&control)
        .await
        .unwrap();
    assert_eq!(nothing, cleared);
    let after = meshway_control::shared::select_profile(&control, "local".into())
        .await
        .unwrap();
    assert_eq!(after.version.map(|v| v.get()), Some(5));
    let control: Control = control.into_inner();
    control.shutdown().await.unwrap();
    server.close().await;
}
