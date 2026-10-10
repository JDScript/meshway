//! Netmap receipts keep the K2 rules: the current instance, saved before
//! applied within one attempt, immutable sequences, history for a netmap the
//! Node once held, and applied only for its current netmap.
use super::*;
use meshway_protocol::{
    execution::{ReceiptFreshness, ReceiptPhase, ReceiptRequest},
    live::{ApplicationPhase, ConfigurationApplyProgress},
};

fn receipt(
    instance: &InstanceRequest,
    netmap: NetmapRef,
    phase: ReceiptPhase,
    attempt: u64,
    sequence: u64,
) -> ReceiptRequest {
    ReceiptRequest {
        instance: instance.clone(),
        netmap,
        phase,
        attempt: Version::new(attempt).unwrap(),
        sequence: Sequence::new(sequence).unwrap(),
        reported_at_ms: chrono::Utc::now().timestamp_millis() as u64,
        failure: None,
    }
}

/// Grants a→b `udp:53` too: a new serial of a's (and b's) netmap.
async fn change_access(lab: &Lab) {
    let current = lab
        .app
        .configurations()
        .read(&lab.owner, lab.network, None)
        .await
        .unwrap();
    let mut config = current.document.configuration.clone();
    config.access[0].ip.push("udp:53".into());
    lab.app
        .configurations()
        .apply(
            &lab.owner,
            lab.network,
            meshway_protocol::configuration::ConfigurationRequest::Document {
                expected_revision: current.revision,
                request_id: rid(),
                source_jsonc: WireSecret::new(canonical_json(&config).unwrap().expose().into()),
            },
            rid(),
        )
        .await
        .unwrap();
}

async fn progress(lab: &Lab, revision: Revision) -> ConfigurationApplyProgress {
    let identity = crate::state::service::ReadIdentity::user_session(&lab.owner, lab.network)
        .unwrap();
    let provider = std::sync::Arc::clone(&lab.app.user_state_service().unwrap().provider);
    lab.app
        .executor
        .handle()
        .read(move |tx| provider.progress(tx, &identity, revision, chrono::Utc::now()))
        .await
        .unwrap()
}

#[tokio::test]
async fn a_replaced_instance_cannot_receipt_and_the_new_one_starts_a_new_attempt() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let lab = lab(false).await;
    let execution = lab.app.execution();
    let (token, old, netmap) = (&lab.tokens[0], &lab.instances[0], lab.applied[0]);
    execution
        .receipt(token, receipt(old, netmap, ReceiptPhase::Saved, 1, 0))
        .await
        .unwrap();
    let lease = execution
        .acquire(
            token,
            AcquireInstanceRequest {
                instance_id: iid(),
                capabilities: caps(),
            },
        )
        .await
        .unwrap();
    let new = InstanceRequest {
        instance_id: lease.instance_id,
        generation: lease.generation,
    };
    assert!(matches!(
        execution
            .receipt(token, receipt(old, netmap, ReceiptPhase::Applied, 1, 1))
            .await,
        Err(Error::ReceiptInstanceMismatch)
    ));
    // The same installed netmap, receipted again by the new instance.
    execution
        .receipt(token, receipt(&new, netmap, ReceiptPhase::Saved, 1, 0))
        .await
        .unwrap();
    execution
        .receipt(token, receipt(&new, netmap, ReceiptPhase::Applied, 1, 1))
        .await
        .unwrap();
    let view = progress(&lab, netmap.revision).await;
    assert_eq!(
        view.nodes[&lab.nodes[0]].instance_id,
        Some(new.instance_id)
    );
    lab.app.shutdown().await.unwrap();
}

#[tokio::test]
async fn receipts_order_saved_before_applied_and_only_for_the_current_netmap() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let lab = lab(false).await;
    let execution = lab.app.execution();
    let (token, instance, netmap) = (&lab.tokens[0], &lab.instances[0], lab.applied[0]);
    let before = lab.references().await;
    // Applied needs an earlier saved in the same attempt.
    assert!(matches!(
        execution
            .receipt(token, receipt(instance, netmap, ReceiptPhase::Applied, 1, 1))
            .await,
        Err(Error::ReceiptPrecondition)
    ));
    execution
        .receipt(token, receipt(instance, netmap, ReceiptPhase::Saved, 1, 0))
        .await
        .unwrap();
    let applied = receipt(instance, netmap, ReceiptPhase::Applied, 1, 1);
    execution.receipt(token, applied.clone()).await.unwrap();
    // An identical retry keeps the original; other content is a conflict.
    execution.receipt(token, applied.clone()).await.unwrap();
    let mut changed = applied.clone();
    changed.reported_at_ms += 1;
    assert!(matches!(
        execution.receipt(token, changed).await,
        Err(Error::ReceiptContentConflict)
    ));
    // A netmap this Node never held is not history.
    let mut foreign = netmap;
    foreign.serial = Version::new(netmap.serial.get() + 40).unwrap();
    assert!(matches!(
        execution
            .receipt(token, receipt(instance, foreign, ReceiptPhase::Saved, 2, 0))
            .await,
        Err(Error::ReceiptTargetMismatch)
    ));
    // Receipts never change any netmap.
    assert_eq!(lab.references().await, before);
    let revision = netmap.revision;
    let view = progress(&lab, revision).await;
    let node = &view.nodes[&lab.nodes[0]];
    assert_eq!(node.target, netmap);
    assert_eq!(node.phase, ApplicationPhase::Applied);
    assert_eq!(node.freshness, ReceiptFreshness::Current);

    // A new serial supersedes the old netmap: late saved evidence stays
    // history, applied is refused, and the old revision's progress says so.
    change_access(&lab).await;
    let next = lab.netmap(lab.nodes[0]).await.reference();
    assert!(next.serial > netmap.serial);
    execution
        .receipt(token, receipt(instance, netmap, ReceiptPhase::Saved, 2, 0))
        .await
        .unwrap();
    assert!(matches!(
        execution
            .receipt(token, receipt(instance, netmap, ReceiptPhase::Applied, 2, 1))
            .await,
        Err(Error::ReceiptNetmapSuperseded)
    ));
    assert_eq!(
        progress(&lab, revision).await.nodes[&lab.nodes[0]].freshness,
        ReceiptFreshness::Superseded
    );
    execution
        .receipt(token, receipt(instance, next, ReceiptPhase::Saved, 1, 0))
        .await
        .unwrap();
    execution
        .receipt(token, receipt(instance, next, ReceiptPhase::Applied, 1, 1))
        .await
        .unwrap();
    lab.app.shutdown().await.unwrap();
}
