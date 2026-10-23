//! X12: a failed capture change proves neither outcome; the owner reads
//! the table back and completes it from the target or the start.
use super::*;

/// X12: one failed change of each kind, injected as the kernel applying it
/// with its tool and the next read failing (`lost`), and as the kernel never
/// seeing it (`failing`): the owner reads the table back and completes the
/// change from the target or the start; a read-back that is neither fails
/// closed at once.
#[tokio::test]
#[ignore = "requires an isolated Linux network namespace, TUN, nft and NET_ADMIN"]
async fn failed_capture_changes_are_read_back_and_completed() {
    use crate::linux::kernel::tool::command::faults::{FAILING_CHANGES, LOST_CHANGES};
    use crate::linux::kernel::verification::{Boxed, Owner, Reread, retried};
    use std::sync::atomic::Ordering::SeqCst;
    assert_eq!(
        std::env::var("MESHWAY_UNDERLAY_ISOLATED").as_deref(),
        Ok("1")
    );
    // The Kernel owns the client TUN and the monitor: kept to the end.
    let Setup {
        _temp,
        tasks,
        kernel: _kernel,
        mut backend,
        device,
        mesh,
        ..
    } = setup().await;
    let (changes, _) = tokio::sync::watch::channel(None);
    let stop = tokio_util::sync::CancellationToken::new();
    let stops = [&stop];
    fn reread(backend: &mut Backend) -> Boxed<'_, Reread> {
        Box::pin(backend.reread())
    }
    /// A change as the owner's retry takes it.
    fn change<F>(run: F) -> F
    where
        F: for<'a> FnMut(&'a mut Backend) -> Boxed<'a, crate::Result<()>>,
    {
        run
    }
    let stage = |plan: EgressCapturePlan, mesh: Ranges| {
        change(move |backend: &mut Backend| {
            let (plan, mesh) = (plan.clone(), mesh.clone());
            Box::pin(async move { backend.stage(plan, mesh).await })
        })
    };
    let switch = |plan: EgressCapturePlan, mesh: Ranges, device: Device| {
        change(move |backend: &mut Backend| {
            let (plan, mesh, device) = (plan.clone(), mesh.clone(), device.clone());
            Box::pin(async move {
                backend.stage(plan.clone(), mesh.clone()).await?;
                backend
                    .commit_staged(plan, mesh, Some(device), || Ok(()))
                    .await
            })
        })
    };
    fn pause(backend: &mut Backend) -> Boxed<'_, crate::Result<()>> {
        Box::pin(backend.pause())
    }
    let mut outcomes = Vec::new();
    for (operation, lost) in [
        ("stage", true),
        ("stage", false),
        // Each pause follows a switch: it has a device's routes to remove.
        ("switch", true),
        ("pause", true),
        ("switch", false),
        ("pause", false),
    ] {
        // Each stage widens the staged union, so each one changes the sets.
        let plan = match (operation, lost) {
            ("stage", true) => narrow("192.0.2.7".parse().unwrap(), "192.0.2.8".parse().unwrap()),
            ("stage", false) => narrow("192.0.2.9".parse().unwrap(), "192.0.2.8".parse().unwrap()),
            _ => full(),
        };
        // The bare change fails; its read-back names where the kernel is.
        if lost {
            LOST_CHANGES.store(1, SeqCst);
        } else {
            FAILING_CHANGES.store(1, SeqCst);
        }
        let failed = match operation {
            "stage" => stage(plan.clone(), mesh.clone())(&mut backend).await,
            "switch" => switch(plan.clone(), mesh.clone(), device.clone())(&mut backend).await,
            _ => backend.pause().await,
        };
        assert!(failed.is_err(), "{operation} lost={lost}");
        let observed = backend.reread().await;
        // A lost replacement of a gate without routes (stage, and the stage
        // step a switch begins with) is at its target; one of a gate with
        // routes (pause after the switch) is on its own path.
        let expected = lost && operation != "pause";
        assert!(
            matches!(
                (&observed, expected),
                (Reread::Done, true) | (Reread::Start, false)
            ),
            "{operation} lost={lost}: {observed}"
        );
        // The owner's retry completes it from there; a change the kernel
        // never saw fails once more first, read back and run again after 1 s.
        if !lost {
            FAILING_CHANGES.store(1, SeqCst);
        }
        let result = match operation {
            "stage" => {
                let run = stage(plan.clone(), mesh.clone());
                retried(
                    &mut backend,
                    Owner::Egress,
                    operation,
                    &changes,
                    &stops,
                    run,
                    reread,
                )
                .await
            }
            "switch" => {
                let run = switch(plan.clone(), mesh.clone(), device.clone());
                retried(
                    &mut backend,
                    Owner::Egress,
                    operation,
                    &changes,
                    &stops,
                    run,
                    reread,
                )
                .await
            }
            _ => {
                retried(
                    &mut backend,
                    Owner::Egress,
                    operation,
                    &changes,
                    &stops,
                    pause,
                    reread,
                )
                .await
            }
        };
        assert!(result.is_ok(), "{operation} lost={lost}: {result:?}");
        assert!(
            matches!(
                backend.check().await,
                crate::linux::kernel::verification::Verification::Holds
            ),
            "{operation} lost={lost}"
        );
        assert!(backend.ledger.pending_gate.is_none());
        assert_eq!(
            backend.ledger.gate.device.is_some(),
            operation == "switch",
            "{operation} lost={lost}"
        );
        outcomes.push(format!(
            "{operation}-{}-{observed}",
            if lost { "lost" } else { "failing" }
        ));
        assert!(changes.borrow().is_none());
    }
    assert_eq!(LOST_CHANGES.load(SeqCst) + FAILING_CHANGES.load(SeqCst), 0);
    // A failed pause whose read-back differs (a rule of the table removed
    // by someone else) is not retried: it ends kernel ownership as before.
    FAILING_CHANGES.store(1, SeqCst);
    assert!(
        switch(full(), mesh.clone(), device.clone())(&mut backend)
            .await
            .is_err()
    );
    let table = backend.ledger.program.table();
    let listed = std::process::Command::new("/usr/sbin/nft")
        .args(["-j", "list", "table", "inet", &table])
        .output()
        .unwrap();
    let listed: serde_json::Value = serde_json::from_slice(&listed.stdout).unwrap();
    let rule = listed["nftables"]
        .as_array()
        .unwrap()
        .iter()
        .find_map(|entry| entry.get("rule"))
        .unwrap()
        .clone();
    let handle = rule["handle"].as_u64().unwrap().to_string();
    let deleted = std::process::Command::new("/usr/sbin/nft")
        .args([
            "delete",
            "rule",
            "inet",
            &table,
            rule["chain"].as_str().unwrap(),
            "handle",
            &handle,
        ])
        .status()
        .unwrap();
    assert!(deleted.success());
    let result = retried(
        &mut backend,
        Owner::Egress,
        "pause",
        &changes,
        &stops,
        pause,
        reread,
    )
    .await;
    assert!(
        matches!(result, Err((_, "verify"))),
        "a changed table is not retried: {result:?}"
    );
    println!(
        "X12_EGRESS_CHANGES={},foreign-rule-verify",
        outcomes.join(",")
    );
    tasks.shutdown().await.unwrap();
}

/// U3 C: a retirement whose table deletion succeeded but whose read-back
/// failed recorded `retiring` before deleting: the next recovery (a new
/// term, `down` again, a process start) reads the table gone with every
/// rule and route already removed and completes the retirement. A table
/// that disappears outside a retirement stays a foreign removal. Before:
/// "receipt present, table absent" was a permanent conflict.
#[tokio::test]
#[ignore = "requires an isolated Linux network namespace, TUN, nft and NET_ADMIN"]
async fn a_retirement_whose_read_back_failed_completes_and_a_missing_table_otherwise_conflicts() {
    use crate::linux::kernel::tool::command::faults::FAILING_RETIRE_READS;
    use std::sync::atomic::Ordering::SeqCst;
    assert_eq!(
        std::env::var("MESHWAY_UNDERLAY_ISOLATED").as_deref(),
        Ok("1")
    );
    let tables = || {
        String::from_utf8(
            std::process::Command::new("/usr/sbin/nft")
                .args(["list", "tables"])
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap()
    };
    let Setup {
        _temp,
        tasks,
        mut kernel,
        mut backend,
        mesh,
        ..
    } = setup().await;
    backend.stage(full(), mesh.clone()).await.unwrap();
    assert!(!backend.ledger.rules.is_empty());
    let table = backend.ledger.program.table();
    FAILING_RETIRE_READS.store(1, SeqCst);
    assert!(matches!(backend.retire().await, Err(Error::Netlink)));
    assert!(!tables().contains(&table), "the deletion took");
    assert!(backend.ledger.retiring && !backend.ledger.removed);
    drop(backend);
    let directory = kernel.store.directory();
    recover(&kernel.handle, directory.clone(), 65_536, false)
        .await
        .unwrap();
    let (_, ledger) = Store::open(directory.clone()).await.unwrap();
    assert!(
        ledger.is_none(),
        "completed: the removed record is archived"
    );
    println!("U3_EGRESS_RETIRE=read-back-failed,completed-by-recovery");
    kernel.down().await.unwrap();
    drop(kernel);
    tasks.shutdown().await.unwrap();
    drop(_temp);

    // Outside a retirement, a missing table is someone else's removal.
    let Setup {
        _temp,
        tasks,
        kernel,
        mut backend,
        mesh,
        ..
    } = setup().await;
    backend.stage(full(), mesh).await.unwrap();
    let table = backend.ledger.program.table();
    let status = std::process::Command::new("/usr/sbin/nft")
        .args(["delete", "table", "inet", &table])
        .status()
        .unwrap();
    assert!(status.success());
    drop(backend);
    assert!(matches!(
        recover(&kernel.handle, kernel.store.directory(), 65_536, false).await,
        Err(Error::Conflict)
    ));
    println!("U3_EGRESS_FOREIGN=conflict");
    drop(kernel);
    tasks.shutdown().await.unwrap();
}
