//! Netmaps are stored in the transactions that change stable inputs and are
//! distributed by the long-poll `sync`; real App, enrollment and commits.
use super::node::{caps, fixture, session};
use super::*;
use crate::accounts::{Secret, random_uuid};
use crate::store::netmap::Current;
use meshway_configuration::netmap::CompiledNetmap;
use meshway_configuration::*;
use meshway_model::*;
use meshway_protocol::configuration::ConfigurationRequest;
use meshway_protocol::netmap::{SyncNetmap, SyncRequest, SyncResponse};
use meshway_protocol::{WireSecret, execution::*, node::NodeSessionResponse};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

mod commit;
mod signing;
mod sync;

fn rid() -> RequestId {
    RequestId::new(random_uuid().unwrap()).unwrap()
}

async fn owner(app: &App) -> String {
    app.accounts()
        .login(
            "owner@example.com".into(),
            Secret::new("a sufficiently long password".into()),
            rid(),
        )
        .await
        .unwrap()
        .token
        .expose()
        .to_owned()
}

async fn revision(app: &App, owner: &str, network: NetworkId) -> Revision {
    app.networks()
        .get(owner, network)
        .await
        .unwrap()
        .current_revision
}

async fn edit(
    app: &App,
    owner: &str,
    network: NetworkId,
    ops: Vec<ConfigurationOperation>,
) -> Result<Revision, crate::error::Error> {
    let expected_revision = revision(app, owner, network).await;
    app.configurations()
        .apply(
            owner,
            network,
            ConfigurationRequest::Operations {
                expected_revision,
                request_id: rid(),
                ops,
            },
            rid(),
        )
        .await
        .map(|response| response.revision)
}

/// A new revision whose canonical configuration is unchanged: only a comment.
async fn comment_only_edit(app: &App, owner: &str, network: NetworkId) -> Revision {
    let current = app
        .configurations()
        .read(owner, network, None)
        .await
        .unwrap();
    let response = app
        .configurations()
        .apply(
            owner,
            network,
            ConfigurationRequest::Document {
                expected_revision: current.revision,
                request_id: rid(),
                source_jsonc: WireSecret::new(format!(
                    "// unrelated {}\n{}",
                    rid(),
                    current.document.source_jsonc.expose()
                )),
            },
            rid(),
        )
        .await
        .unwrap();
    assert!(response.revision > current.revision);
    response.revision
}

fn rename(node: NodeId, hostname: &str) -> ConfigurationOperation {
    ConfigurationOperation::SetNodeSettings {
        node_id: node,
        fields: NodeSettingsPatch {
            hostname: Input::Value(hostname.into()),
            ..Default::default()
        },
    }
}

async fn current(app: &App, network: NetworkId, node: NodeId) -> Option<Current> {
    app.executor
        .handle()
        .read(move |tx| crate::store::netmap::current(tx, network, node))
        .await
        .unwrap()
}

async fn load(app: &App, network: NetworkId, node: NodeId) -> CompiledNetmap {
    let reference = current(app, network, node).await.unwrap().current;
    let sealer = app.sealer();
    app.executor
        .handle()
        .read(move |tx| crate::store::netmap::load(tx, &sealer, network, node, reference))
        .await
        .unwrap()
}

/// Every stored row as (node, serial, digest, withdrawn), to prove that
/// nothing was written.
async fn rows(app: &App, network: NetworkId) -> Vec<(Vec<u8>, i64, Vec<u8>, bool)> {
    app.executor
        .handle()
        .read(move |tx| {
            let mut statement = tx.prepare(
                "SELECT node_id,serial,digest,withdrawn_at IS NOT NULL FROM node_netmaps
                 WHERE network_id=?1 ORDER BY node_id,serial",
            )?;
            let rows = statement
                .query_map([network.as_uuid().as_bytes().as_slice()], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await
        .unwrap()
}

fn serial(current: Option<Current>) -> u64 {
    current.unwrap().current.serial.get()
}

async fn acquire(app: &App, token: &NodeSessionResponse) -> InstanceLease {
    app.execution()
        .acquire(
            token.token.expose(),
            AcquireInstanceRequest {
                instance_id: InstanceId::new(random_uuid().unwrap()).unwrap(),
                capabilities: caps(),
            },
        )
        .await
        .unwrap()
}

fn holding(netmap: Option<NetmapRef>) -> SyncRequest {
    SyncRequest {
        netmap,
        state: None,
        server_key_generation: None,
        traversal_requests: Default::default(),
        associations: None,
    }
}

/// Spawns a sync held by the App's own service.
fn spawn_sync(
    app: &App,
    token: &NodeSessionResponse,
    request: SyncRequest,
) -> tokio::task::JoinHandle<(Result<SyncResponse, crate::error::Error>, Duration)> {
    let service = app.netmaps();
    let token = token.token.expose().to_owned();
    tokio::spawn(async move {
        let mut request = request;
        if request.netmap.is_some() && request.state.is_none() {
            let initial = service.sync(&token, holding(None)).await.unwrap();
            request.state = initial.peer_facts.map(|facts| facts.cursor);
        }
        let started = Instant::now();
        let response = service.sync(&token, request).await;
        (response, started.elapsed())
    })
}

/// Waits until `count` syncs are held by the hub.
async fn held(app: &App, count: usize) {
    for _ in 0..200 {
        if app.effects.netmaps.pending() == count {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("sync was not held");
}
