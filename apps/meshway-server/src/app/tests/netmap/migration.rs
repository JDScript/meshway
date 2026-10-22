//! A database written by a real App, in the form schema 8 stores it, is
//! migrated by opening: the same password logs in, the same session and Node
//! credentials authenticate, every sealed configuration version opens to the
//! same document, signing keys sign, and no netmap serial goes back.
use super::*;
use crate::settings::Migration;

/// Rows per table, without the migration record and the catalog checks the
/// resource scheduler of every running App writes on its own.
async fn counts(app: &App) -> Vec<(String, i64)> {
    app.executor
        .handle()
        .read(|tx| {
            let tables: Vec<String> = tx
                .prepare("SELECT name FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\' AND name NOT IN ('schema_migrations','rule_set_catalog_checks') ORDER BY name")?
                .query_map([], |row| row.get(0))?
                .collect::<Result<_, _>>()?;
            let mut counts = Vec::new();
            for table in tables {
                let count = tx.query_row(&format!("SELECT count(*) FROM \"{table}\""), [], |row| row.get(0))?;
                counts.push((table, count));
            }
            Ok(counts)
        })
        .await
        .unwrap()
}

async fn documents(app: &App, network: NetworkId) -> Vec<ConfigurationDocument> {
    let sealer = app.sealer();
    app.executor
        .handle()
        .read(move |tx| {
            let current = crate::store::configuration::versions::current_revision(tx, network)?;
            (1..=current.get())
                .map(|revision| {
                    let revision = Revision::new(revision).unwrap();
                    crate::store::configuration::versions::load(tx, network, revision)?
                        .open(&sealer)
                        .map(|snapshot| snapshot.document)
                })
                .collect()
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn a_schema_8_database_opens_migrated_with_its_credentials_secrets_and_serials() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (dir, app, network, a, b, ak, _, signer_a, _) = fixture().await;
    let owner = owner(&app).await;
    let node = session(&app.execution(), network, a, &ak, &signer_a).await;
    let SyncNetmap::Full { .. } = app
        .netmaps()
        .sync(node.token.expose(), holding(None))
        .await
        .unwrap()
        .netmap
    else {
        panic!("expected full");
    };
    let current_revision = revision(&app, &owner, network).await;
    let (before, documents_before) = (counts(&app).await, documents(&app, network).await);
    let serials = (
        serial(current(&app, network, a).await),
        serial(current(&app, network, b).await),
    );
    app.shutdown().await.unwrap();
    let path = dir.path().canonicalize().unwrap().join("data");
    crate::store::schema::migrate::tests::downgrade_to_v8(&path);
    let database = path.join("controller.sqlite");
    let original = std::fs::read(&database).unwrap();

    // Offline maintenance never migrates implicitly.
    assert_eq!(
        App::open(Settings::new(&path)).await.err(),
        Some(AppError::MigrationRequired)
    );
    assert_eq!(std::fs::read(&database).unwrap(), original);

    let mut settings = Settings::new(&path);
    settings.migration = Migration::Automatic;
    let app = App::open(settings).await.unwrap();
    let migrated = app.migrated().unwrap().clone();
    assert_eq!((migrated.from, migrated.to), (8, 9));
    assert_eq!(std::fs::read(&migrated.backup).unwrap(), original);
    assert_eq!(counts(&app).await, before);
    assert_eq!(documents(&app, network).await, documents_before);
    assert_eq!(revision(&app, &owner, network).await, current_revision);
    // The session issued before still authenticates; the password logs in.
    app.accounts().authenticate(&owner).await.unwrap();
    super::owner(&app).await;
    // Same build: nothing to compile again, no serial changed or went back.
    assert_eq!(
        (
            serial(current(&app, network, a).await),
            serial(current(&app, network, b).await)
        ),
        serials
    );
    // The Node's credentials and the sealed signing key still work.
    let node = session(&app.execution(), network, a, &ak, &signer_a).await;
    let response = app
        .netmaps()
        .sync(node.token.expose(), holding(None))
        .await
        .unwrap();
    let SyncNetmap::Full { netmap } = response.netmap else {
        panic!("expected full");
    };
    assert_eq!(netmap.netmap.serial.get(), serials.0);
    assert_eq!(response.signature.key_generation.get(), 1);
    app.shutdown().await.unwrap();

    // Reopened, it is a current database; the next opening migrates nothing.
    let app = App::open(Settings::new(&path)).await.unwrap();
    assert_eq!(app.migrated(), None);
    app.shutdown().await.unwrap();
}
