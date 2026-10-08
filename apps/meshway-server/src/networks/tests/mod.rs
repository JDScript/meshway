use super::*;
use crate::{
    App, Settings,
    accounts::{IssuedSession, Secret, TEST_KDF_LOCK, random_uuid},
    secrets::{CursorScope, Purpose},
};
use meshway_configuration::{AddressPools, canonical_json, new_network};
use meshway_model::{MembershipRole, NetworkActions, Revision, UserId};
use rusqlite::params;
use std::{
    future::Future,
    path::PathBuf,
    pin::Pin,
    sync::mpsc,
    task::{Context, Poll, Waker},
};
use tokio::sync::oneshot;

fn request() -> RequestId {
    RequestId::new(random_uuid().unwrap()).unwrap()
}
async fn fixture() -> (tempfile::TempDir, PathBuf, App) {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().canonicalize().unwrap().join("data");
    let app = App::open(Settings::new(&path)).await.unwrap();
    (temp, path, app)
}
async fn user(app: &App, name: &str) -> IssuedSession {
    app.accounts()
        .register(
            format!("{name}@example.test"),
            Secret::new("valid password".into()),
            request(),
        )
        .await
        .unwrap()
}
fn pending<F: Future>(future: Pin<&mut F>) {
    assert!(matches!(
        future.poll(&mut Context::from_waker(Waker::noop())),
        Poll::Pending
    ));
}
async fn grant(networks: &Networks, network: NetworkId, user: UserId, role: &'static str) {
    networks.handle.execute(move |connection| {
        connection.execute("INSERT INTO memberships (id, network_id, user_id, role, version, joined_at) VALUES (?1, ?2, ?3, ?4, 1, ?5)", params![random_uuid().unwrap().as_bytes().as_slice(), network.as_uuid().as_bytes().as_slice(), user.as_uuid().as_bytes().as_slice(), role, Utc::now().timestamp_millis()]).unwrap();
    }).await.unwrap();
}
async fn config_bytes(networks: &Networks, id: NetworkId) -> (Vec<u8>, Vec<u8>, Vec<u8>, i64, i64) {
    networks.handle.execute(move |connection| connection.query_row("SELECT c.source_jsonc, c.configuration, c.bindings, n.current_revision, n.publication_sequence FROM networks n JOIN configuration_versions c ON c.network_id = n.id AND c.revision = n.current_revision WHERE n.id = ?1", [id.as_uuid().as_bytes().as_slice()], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?))).unwrap()).await.unwrap()
}

mod creation;

mod permissions;
