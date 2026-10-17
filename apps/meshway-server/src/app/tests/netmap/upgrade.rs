//! S4: the Server's build is a netmap compiler input. A start by the same
//! build writes nothing; a start by another build compiles every stored
//! netmap again before any request, and a Node's next `sync` receives the
//! content that changed.
use super::*;

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
