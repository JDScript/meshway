use super::*;
use crate::accounts::{Secret, random_uuid};
use crate::{App, Settings};
use meshway_model::RequestId;
use std::os::unix::fs::PermissionsExt;

thread_local! {
    /// A file another writer leaves in the output, then a failed copy.
    static INTRUDE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub(super) fn after_copy(output: &Path) -> Result<(), AppError> {
    if INTRUDE.get() {
        std::fs::write(output.join("foreign"), b"not the backup's").unwrap();
        return Err(AppError::BackupOutput);
    }
    Ok(())
}

fn request() -> RequestId {
    RequestId::new(random_uuid().unwrap()).unwrap()
}

fn mode(path: &Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

const EMAIL: &str = "backup@example.test";
const PASSWORD: &str = "valid backup password";

async fn installation(root: &Path) -> PathBuf {
    let data = root.join("data");
    let app = App::open(Settings::new(data.clone())).await.unwrap();
    app.accounts()
        .register(EMAIL.into(), Secret::new(PASSWORD.into()), request())
        .await
        .unwrap();
    app.shutdown().await.unwrap();
    data
}

#[tokio::test]
async fn a_backup_is_a_private_data_directory_that_opens_with_the_same_accounts() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let root = tempfile::tempdir().unwrap();
    let root = root.path().canonicalize().unwrap();
    let data = installation(&root).await;
    let source = std::fs::read(data.join("controller.sqlite")).unwrap();
    let output = root.join("backup");

    let backup = backup(data.clone(), output.clone()).await.unwrap();
    assert_eq!(backup.output, output);
    assert_eq!(backup.schema, schema::VERSION);
    assert!(backup.database_bytes > 0);
    assert_eq!(
        std::fs::read(data.join("controller.sqlite")).unwrap(),
        source,
        "the source is not written"
    );
    assert_eq!(mode(&output), 0o700);
    let mut names: Vec<_> = std::fs::read_dir(&output)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(
        names,
        ["controller.lock", "controller.sqlite", "master-key"]
    );
    for name in &names {
        assert_eq!(mode(&output.join(name)), 0o600, "{name}");
    }
    assert_eq!(
        std::fs::read(output.join("master-key")).unwrap(),
        std::fs::read(data.join("master-key")).unwrap()
    );

    let restored = App::open(Settings::new(output)).await.unwrap();
    restored
        .accounts()
        .login(EMAIL.into(), Secret::new(PASSWORD.into()), request())
        .await
        .unwrap();
    restored.check_storage().await.unwrap();
    restored.shutdown().await.unwrap();
}

#[tokio::test]
async fn an_existing_output_or_a_running_server_writes_nothing() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let root = tempfile::tempdir().unwrap();
    let root = root.path().canonicalize().unwrap();
    let data = installation(&root).await;

    let existing = root.join("existing");
    std::fs::create_dir(&existing).unwrap();
    assert_eq!(
        backup(data.clone(), existing.clone()).await,
        Err(AppError::BackupOutput)
    );
    assert_eq!(std::fs::read_dir(&existing).unwrap().count(), 0);
    // A private directory of the same user with its own files is not adopted.
    let private = root.join("private");
    std::fs::create_dir(&private).unwrap();
    std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::write(private.join("keep"), b"kept").unwrap();
    assert_eq!(
        backup(data.clone(), private.clone()).await,
        Err(AppError::BackupOutput)
    );
    assert_eq!(std::fs::read(private.join("keep")).unwrap(), b"kept");
    assert_eq!(std::fs::read_dir(&private).unwrap().count(), 1);

    let running = App::open(Settings::new(data.clone())).await.unwrap();
    let output = root.join("while-running");
    assert_eq!(
        backup(data.clone(), output.clone()).await,
        Err(AppError::AlreadyRunning)
    );
    assert!(!output.exists());
    running.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_data_directory_without_a_database_has_nothing_to_back_up() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path().canonicalize().unwrap();
    let output = root.join("backup");
    assert_eq!(
        backup(root.join("empty"), output.clone()).await,
        Err(AppError::UnsupportedSchema)
    );
    assert!(!output.exists());
}

#[tokio::test]
async fn a_mismatched_master_key_is_refused_before_anything_is_copied() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let root = tempfile::tempdir().unwrap();
    let root = root.path().canonicalize().unwrap();
    let data = installation(&root).await;
    std::fs::create_dir(root.join("other")).unwrap();
    let other = installation(&root.join("other")).await;
    std::fs::copy(other.join("master-key"), data.join("master-key")).unwrap();
    let output = root.join("backup");
    assert_eq!(backup(data, output.clone()).await, Err(AppError::MasterKey));
    assert!(!output.exists());
}

/// A failed backup removes what it wrote and the directory it created; a
/// file someone else put there meanwhile stays, and so does the directory.
#[tokio::test]
async fn a_failed_backup_removes_only_what_it_wrote() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let root = tempfile::tempdir().unwrap();
    let root = root.path().canonicalize().unwrap();
    let data = installation(&root).await;
    let output = root.join("backup");
    let data_dir = data.clone();
    let target = output.clone();
    let failed = tokio::task::spawn_blocking(move || {
        INTRUDE.set(true);
        let result = run(&data_dir, &target);
        INTRUDE.set(false);
        result
    })
    .await
    .unwrap();
    assert_eq!(failed, Err(AppError::BackupOutput));
    let mut names: Vec<_> = std::fs::read_dir(&output)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(names, ["foreign"]);
    std::fs::remove_file(output.join("foreign")).unwrap();
    std::fs::remove_dir(&output).unwrap();
    // Without the intruder the same backup succeeds.
    backup(data, output.clone()).await.unwrap();
}
