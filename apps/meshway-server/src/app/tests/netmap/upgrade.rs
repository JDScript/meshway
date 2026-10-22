//! S4: the Server's build is a netmap compiler input. A start by the same
//! build writes nothing; a start by another build compiles every stored
//! netmap again before any request, and a Node's next `sync` receives the
//! content that changed.
use super::*;
use meshway_model::state::{ConfigurationState, NodeState, SelectionState, SystemState};
use meshway_protocol::state::StateReport;

async fn reopen(dir: &tempfile::TempDir, app: App) -> App {
    app.shutdown().await.unwrap();
    App::open(Settings::new(
        dir.path().canonicalize().unwrap().join("data"),
    ))
    .await
    .unwrap()
}

async fn inputs(app: &App, network: NetworkId) -> Vec<(Vec<u8>, Vec<u8>)> {
    app.executor
        .handle()
        .read(move |tx| {
            let mut statement = tx.prepare(
                "SELECT node_id,digest FROM node_netmap_inputs WHERE network_id=?1 ORDER BY node_id",
            )?;
            let rows = statement
                .query_map([network.as_uuid().as_bytes().as_slice()], |row| {
                    Ok((row.get(0)?, row.get(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn a_start_by_another_build_recompiles_and_the_nodes_next_sync_receives_it() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (dir, app, network, a, b, ak, _, signer_a, _) = fixture().await;
    let token = session(&app.execution(), network, a, &ak, &signer_a).await;
    let SyncNetmap::Full { netmap: compiled } = app
        .netmaps()
        .sync(token.token.expose(), holding(None))
        .await
        .unwrap()
        .netmap
    else {
        panic!("expected full");
    };

    // The same build: nothing is compiled or written.
    let (stored, fingerprints) = (rows(&app, network).await, inputs(&app, network).await);
    let app = reopen(&dir, app).await;
    assert_eq!(rows(&app, network).await, stored);
    assert_eq!(inputs(&app, network).await, fingerprints);

    // What an earlier build left: other fingerprints, and for `a` content
    // it compiled differently (here a selection version this build does
    // not produce), under the next serial.
    let mut earlier = (*compiled.netmap).clone();
    earlier.serial = earlier.serial.checked_next().unwrap();
    earlier
        .projection
        .selection_versions
        .insert(a, Version::new(7).unwrap());
    let earlier = CompiledNetmap {
        digest: earlier.content_digest().unwrap(),
        netmap: std::sync::Arc::new(earlier),
    };
    let held = earlier.reference();
    let sealer = app.sealer();
    app.executor
        .handle()
        .write(move |tx| {
            let now = crate::store::execution::now()?;
            crate::store::netmap::store_as_another_build(tx, &sealer, network, a, &earlier, now)
        })
        .await
        .unwrap();
    assert_eq!(current(&app, network, a).await.unwrap().current, held);
    let b_before = current(&app, network, b).await.unwrap().current;

    let app = reopen(&dir, app).await;
    // `a` is this build's compilation again under a new serial; `b`, whose
    // content this build compiles the same, keeps its serial.
    let refreshed = load(&app, network, a).await;
    assert_eq!(refreshed.netmap.serial.get(), held.serial.get() + 1);
    assert_eq!(refreshed.digest, compiled.digest);
    assert_eq!(current(&app, network, b).await.unwrap().current, b_before);
    assert_eq!(inputs(&app, network).await, fingerprints);

    // `a` still holds the earlier build's netmap: its next sync, under the
    // new process, is answered at once with the new serial.
    let token = session(&app.execution(), network, a, &ak, &signer_a).await;
    let response = app
        .netmaps()
        .sync(token.token.expose(), holding(Some(held)))
        .await
        .unwrap();
    assert_eq!(response.lease.netmap, refreshed.reference());
    assert!(!matches!(response.netmap, SyncNetmap::Unchanged));
    app.shutdown().await.unwrap();
}

/// A report of `applied` as both saved and applied, nothing else.
fn applied(instance: InstanceId, sequence: u64, applied: NetmapRef) -> StateReport {
    let now = chrono::Utc::now();
    StateReport {
        instance_id: instance,
        sequence: Sequence::new(sequence).unwrap(),
        observed_at: now,
        state: NodeState {
            system: SystemState {
                hostname: "upgrade".into(),
                os: "linux".into(),
                arch: "aarch64".into(),
                agent_version: "test".into(),
            },
            capabilities: super::node::caps(),
            configuration: ConfigurationState {
                saved: Some(applied),
                applied: Some(applied),
                progress: None,
            },
            exit_capability: None,
            exit_dns: None,
            listeners: BTreeMap::new(),
            sockets: BTreeMap::new(),
            links: BTreeMap::new(),
            adjacencies: Default::default(),
            paths: BTreeMap::new(),
            selection: SelectionState::default(),
            probes: vec![],
            problems: vec![],
        },
    }
}

/// S5 (OPS18): after a `compilerVersion` upgrade the start compiles every
/// Node again and keeps the earlier compiler's head as the delta base, a
/// payload this build does not decode. The base is history this build
/// cannot read: State of the current netmap is accepted, the Node holding
/// the base is answered in full, and the next change is a delta again.
#[tokio::test]
async fn a_base_another_compiler_wrote_is_no_base_for_state_or_sync() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (dir, app, network, a, _, ak, _, signer_a, _) = fixture().await;
    let token = session(&app.execution(), network, a, &ak, &signer_a).await;
    // The instance's capabilities are a compiler input: acquire first.
    acquire(&app, &token).await;
    let SyncNetmap::Full { netmap: compiled } = app
        .netmaps()
        .sync(token.token.expose(), holding(None))
        .await
        .unwrap()
        .netmap
    else {
        panic!("expected full");
    };
    let mut earlier = (*compiled.netmap).clone();
    earlier.serial = earlier.serial.checked_next().unwrap();
    let sealer = app.sealer();
    let held = app
        .executor
        .handle()
        .write(move |tx| {
            let now = crate::store::execution::now()?;
            crate::store::netmap::store_as_another_compiler(tx, &sealer, network, a, &earlier, now)
        })
        .await
        .unwrap();
    let sealer = app.sealer();
    assert!(
        app.executor
            .handle()
            .read(move |tx| crate::store::netmap::load(tx, &sealer, network, a, held))
            .await
            .is_err(),
        "the fixture must not decode under this build"
    );

    let app = reopen(&dir, app).await;
    let refreshed = load(&app, network, a).await;
    assert_eq!(refreshed.netmap.serial.get(), held.serial.get() + 1);
    assert_eq!(
        current(&app, network, a).await.unwrap(),
        Current {
            current: refreshed.reference(),
            previous: Some(held),
        }
    );

    // State of the current netmap is accepted; one naming the unreadable
    // base is rejected like any netmap that is neither current nor base.
    let token = session(&app.execution(), network, a, &ak, &signer_a).await;
    let lease = acquire(&app, &token).await;
    assert_eq!(
        current(&app, network, a).await.unwrap().current,
        refreshed.reference()
    );
    let states = app.node_state_service().unwrap();
    states
        .accept_report(
            token.token.expose().to_owned(),
            applied(lease.instance_id, 1, refreshed.reference()),
        )
        .await
        .unwrap();
    assert_eq!(
        states
            .accept_report(
                token.token.expose().to_owned(),
                applied(lease.instance_id, 2, held),
            )
            .await
            .unwrap_err(),
        crate::error::Error::InvalidRequest
    );

    // The Node holding the base gets the current netmap in full.
    let response = app
        .netmaps()
        .sync(token.token.expose(), holding(Some(held)))
        .await
        .unwrap();
    let SyncNetmap::Full { netmap } = response.netmap else {
        panic!("expected full, got {:?}", response.netmap);
    };
    assert_eq!(netmap.reference(), refreshed.reference());

    // The next change has this build's base: a delta again.
    let owner = owner(&app).await;
    edit(&app, &owner, network, vec![rename(a, "upgraded")])
        .await
        .unwrap();
    let next = current(&app, network, a).await.unwrap();
    assert_eq!(next.previous, Some(refreshed.reference()));
    let response = app
        .netmaps()
        .sync(token.token.expose(), holding(Some(refreshed.reference())))
        .await
        .unwrap();
    let SyncNetmap::Delta { delta } = response.netmap else {
        panic!("expected delta, got {:?}", response.netmap);
    };
    assert_eq!(
        (delta.base, delta.target),
        (refreshed.reference(), next.current)
    );

    // A start by the same build writes nothing.
    let (stored, fingerprints) = (rows(&app, network).await, inputs(&app, network).await);
    let app = reopen(&dir, app).await;
    assert_eq!(rows(&app, network).await, stored);
    assert_eq!(inputs(&app, network).await, fingerprints);
    app.shutdown().await.unwrap();
}
