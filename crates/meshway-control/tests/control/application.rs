//! Authenticated Control/Server protocol fixtures. Supplied installation results
//! and probe samples exercise recording, not OS installation or packet I/O.
use super::{
    faults::{Fault, Proxy},
    support::{Owner, Server, request},
};
use chrono::Utc;
use meshway_configuration::NetworkConfigurationInput;
use meshway_control::{Control, Error};
use meshway_control_client::{
    ApplyConfigurationPath, GetConfigurationProgressPath, GetConfigurationSourcePath,
    GetConfigurationSourceQuery, GetNodeStatePath,
};
use meshway_model::{NetmapRef, NodeId, Sequence, Version};
use meshway_protocol::{
    ConfigurationRequest, WireSecret,
    execution::ApplyFailure,
    live::{ApplicationPhase, ConfigurationNodeProgress},
    state::NodeStateView,
};
use std::time::Duration;

const RECEIPTS: &str = "/configuration/receipts";

async fn start_control(server: &Server, owner: &Owner, origin: &str, name: &str) -> Control {
    let mut control = server
        .control(owner, origin, name, Default::default())
        .await;
    // Revision progress deliberately retains the revision's initial targets.
    // Publish after acquisition so this fixture's ticket is that exact target,
    // rather than an instance-acquisition refresh of the enrollment revision.
    configure(server, owner, |cfg| {
        cfg.dns.domain = format!("{name}.mesh.example");
    })
    .await;
    crate::support::sync(&mut control).await.unwrap();
    control.renew_instance().await.unwrap();
    control
}

async fn progress(
    server: &Server,
    owner: &Owner,
    node: NodeId,
    target: &NetmapRef,
) -> ConfigurationNodeProgress {
    let mut progress = server
        .client
        .get_configuration_progress(
            &GetConfigurationProgressPath {
                network_id: owner.network,
                revision: target.revision,
            },
            owner.token.expose(),
        )
        .await
        .unwrap();
    let progress = progress.nodes.remove(&node).unwrap();
    assert_eq!(
        &progress.target, target,
        "read the ticket's exact original revision target"
    );
    progress
}

async fn view(server: &Server, owner: &Owner, node: NodeId) -> NodeStateView {
    server
        .client
        .get_node_state(
            &GetNodeStatePath {
                network_id: owner.network,
                node_id: node,
            },
            owner.token.expose(),
        )
        .await
        .unwrap()
        .nodes
        .remove(&node)
        .unwrap()
}

async fn configure(
    server: &Server,
    owner: &Owner,
    edit: impl FnOnce(&mut NetworkConfigurationInput),
) {
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
    let mut input = NetworkConfigurationInput::from(&source.configuration);
    edit(&mut input);
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
        .unwrap_or_else(|error| match error {
            meshway_control_client::ClientError::Http { problem, .. } => panic!("{problem:?}"),
            error => panic!("{error:?}"),
        });
}

fn assert_send<T: Send>(_: &T) {}

#[tokio::test]
async fn lost_saved_and_cancelled_completion_keep_exact_identity_and_order_new_attempts() {
    let server = Server::start().await;
    let owner = server.owner().await;
    let proxy = Proxy::start(&server.origin).await;
    let mut control = start_control(&server, &owner, &proxy.origin, "application-retry").await;
    let node = control.status().binding.unwrap().node_id;
    let target = control.status().held.unwrap();
    proxy.fault(RECEIPTS, Fault::Lost);
    assert!(
        crate::support::acknowledge_saved(&mut control)
            .await
            .is_err()
    );
    let saved = progress(&server, &owner, node, &target).await;
    assert_eq!(saved.phase, ApplicationPhase::Saved);
    assert_eq!(saved.attempt, Some(Version::MIN));
    assert_eq!(saved.sequence, Some(Sequence::MIN));
    tokio::time::sleep(Duration::from_millis(5)).await;
    let first = crate::support::acknowledge_saved(&mut control)
        .await
        .unwrap();
    let duplicate = progress(&server, &owner, node, &target).await;
    assert_eq!(duplicate.received_at, saved.received_at);
    assert_eq!(first.attempt(), Version::MIN);
    assert_eq!(
        crate::support::acknowledge_saved(&mut control)
            .await
            .unwrap()
            .attempt(),
        first.attempt()
    );
    assert_eq!(proxy.count(RECEIPTS), 2);

    let observed = Utc::now();
    proxy.fault(RECEIPTS, Fault::Hold);
    {
        let completion = control.finish_application(&first, Err(ApplyFailure::Tun), observed);
        assert_send(&completion);
        tokio::pin!(completion);
        tokio::select! {
            result = &mut completion => panic!("completion delivered before release: {result:?}"),
            result = tokio::time::timeout(Duration::from_secs(2), proxy.committed()) => result.unwrap(),
        }
        // Drop this waiter after the real database owner has committed.
    }
    proxy.release();
    let failed = progress(&server, &owner, node, &target).await;
    assert_eq!(failed.phase, ApplicationPhase::Failed);
    assert_eq!(failed.attempt, Some(first.attempt()));
    assert_eq!(failed.sequence, Some(Sequence::MIN.checked_next().unwrap()));
    for result in [Ok(()), Err(ApplyFailure::Dns)] {
        assert!(matches!(
            control.finish_application(&first, result, Utc::now()).await,
            Err(Error::Pending)
        ));
    }
    assert_eq!(proxy.count(RECEIPTS), 3);
    control
        .finish_application(
            &first.clone(),
            Err(ApplyFailure::Tun),
            observed + chrono::Duration::seconds(1),
        )
        .await
        .unwrap();
    // Server rejects a same attempt/sequence with a changed reportedAt. Success
    // plus the unchanged receivedAt proves this really was the original retry.
    assert_eq!(
        progress(&server, &owner, node, &target).await.received_at,
        failed.received_at
    );

    let second = crate::support::acknowledge_saved(&mut control)
        .await
        .unwrap();
    assert_eq!(second.attempt(), first.attempt().checked_next().unwrap());
    let saved = progress(&server, &owner, node, &target).await;
    assert_eq!(saved.phase, ApplicationPhase::Saved);
    assert_eq!(saved.attempt, Some(second.attempt()));
    let completed = Utc::now();
    proxy.fault(RECEIPTS, Fault::Lost);
    assert!(
        control
            .finish_application(&second, Ok(()), completed)
            .await
            .is_err()
    );
    let applied = progress(&server, &owner, node, &target).await;
    assert_eq!(applied.phase, ApplicationPhase::Applied);
    assert_eq!(applied.attempt, Some(second.attempt()));
    control
        .finish_application(&second, Ok(()), completed + chrono::Duration::seconds(1))
        .await
        .unwrap();
    assert_eq!(
        progress(&server, &owner, node, &target).await.received_at,
        applied.received_at
    );
    control
        .finish_application(&first, Err(ApplyFailure::Tun), Utc::now())
        .await
        .unwrap();
    let late = progress(&server, &owner, node, &target).await;
    assert_eq!(late.phase, ApplicationPhase::Applied);
    assert_eq!(late.attempt, Some(second.attempt()));
    assert_eq!(late.received_at, applied.received_at);
    assert!(view(&server, &owner, node).await.report.is_none());
    control.shutdown().await.unwrap();
    proxy.close().await;
    server.close().await;
}

#[tokio::test]
async fn tickets_are_bound_to_the_control_owner_and_instance_generation() {
    let server = Server::start().await;
    let owner = server.owner().await;
    let proxy = Proxy::start(&server.origin).await;
    let mut first = start_control(&server, &owner, &proxy.origin, "application-owner").await;
    let ticket = crate::support::acknowledge_saved(&mut first).await.unwrap();
    let mut other = start_control(&server, &owner, &proxy.origin, "application-foreign").await;
    let count = proxy.count(RECEIPTS);
    assert!(matches!(
        other
            .finish_application(&ticket, Err(ApplyFailure::Tun), Utc::now())
            .await,
        Err(Error::Lifecycle)
    ));
    first.stop().await.unwrap();
    first.authenticate().await.unwrap();
    let instance = first.acquire_instance(Default::default()).await.unwrap();
    crate::support::sync(&mut first).await.unwrap();
    assert_ne!(instance.instance_id, ticket.instance().instance_id);
    assert!(instance.generation > ticket.instance().generation);
    assert!(matches!(
        first.finish_application(&ticket, Ok(()), Utc::now()).await,
        Err(Error::Lifecycle)
    ));
    assert_eq!(proxy.count(RECEIPTS), count);
    let recent = crate::support::acknowledge_saved(&mut first).await.unwrap();
    first.shutdown().await.unwrap();
    let mut reopened = Control::open(
        &server.path("application-owner"),
        &proxy.origin,
        meshway_control_client::http_client_builder()
            .unwrap()
            .no_proxy(),
    )
    .unwrap();
    reopened.authenticate().await.unwrap();
    reopened.acquire_instance(Default::default()).await.unwrap();
    crate::support::sync(&mut reopened).await.unwrap();
    let count = proxy.count(RECEIPTS);
    assert!(matches!(
        reopened
            .finish_application(&recent, Err(ApplyFailure::Storage), Utc::now())
            .await,
        Err(Error::Lifecycle)
    ));
    assert_eq!(proxy.count(RECEIPTS), count);
    let fresh = crate::support::acknowledge_saved(&mut reopened)
        .await
        .unwrap();
    assert_eq!(fresh.attempt(), Version::MIN);
    reopened
        .finish_application(&fresh, Ok(()), Utc::now())
        .await
        .unwrap();
    other.shutdown().await.unwrap();
    reopened.shutdown().await.unwrap();
    proxy.close().await;
    server.close().await;
}

#[tokio::test]
async fn receipt_rejections_retire_exact_ticket_without_fencing_and_reapply_new_attempt() {
    let server = Server::start().await;
    let owner = server.owner().await;
    let proxy = Proxy::start(&server.origin).await;
    let mut control = start_control(&server, &owner, &proxy.origin, "receipt-conflicts").await;
    for (code, body_status, refresh) in [
        ("receipt_netmap_superseded", 409, true),
        ("receipt_precondition", 409, true),
        ("receipt_content_conflict", 409, true),
        ("receipt_netmap_superseded", 400, false),
        ("receipt_instance_mismatch", 409, false),
        ("receipt_target_mismatch", 409, false),
        ("request_id_conflict", 409, false),
    ] {
        control.authenticate().await.unwrap();
        control.acquire_instance(Default::default()).await.unwrap();
        crate::support::sync(&mut control).await.unwrap();
        let ticket = crate::support::acknowledge_saved(&mut control)
            .await
            .unwrap();
        let snapshot = control.netmap_snapshot().unwrap();
        let status = control.snapshot();
        proxy.fault(RECEIPTS, Fault::ReceiptConflict(code, body_status));
        let error = control
            .finish_application(&ticket, Ok(()), Utc::now())
            .await
            .unwrap_err();
        assert!(!format!("{error:?}").contains("sensitive receipt body"));
        assert!(!error.to_string().contains("sensitive receipt body"));
        assert_eq!(
            matches!(error, Error::ReceiptRejected(_)),
            refresh,
            "{code}/{body_status}"
        );
        assert_eq!(control.status().authenticated, refresh);
        assert_eq!(status.status().instance.is_some(), refresh);
        assert_eq!(snapshot.ensure_current().is_ok(), refresh);
        if refresh {
            assert_eq!(
                control.status().instance.unwrap().instance_id,
                ticket.instance().instance_id
            );
            let count = proxy.count(RECEIPTS);
            assert!(matches!(
                control
                    .finish_application(&ticket, Ok(()), Utc::now())
                    .await,
                Err(Error::Pending)
            ));
            assert_eq!(
                proxy.count(RECEIPTS),
                count,
                "rejected ticket cannot be resubmitted"
            );
            crate::support::sync(&mut control).await.unwrap();
            let next = crate::support::acknowledge_saved(&mut control)
                .await
                .unwrap();
            assert_eq!(next.attempt(), ticket.attempt().checked_next().unwrap());
            control
                .finish_application(&next, Ok(()), Utc::now())
                .await
                .unwrap();
        } else {
            assert!(control.status().instance.is_none());
        }
    }
    control.shutdown().await.unwrap();
    proxy.close().await;
    server.close().await;
}
