//! Absolute-deadline renewal against real enrollment, sessions and publications.
use super::{
    node::{caps, fixture, session},
    *,
};
use crate::{
    accounts::{Secret, random_uuid},
    error::Error,
    store,
};
use meshway_configuration::{ConfigurationOperation, execution::*};
use meshway_model::{InstanceId, NetworkId, NodeId, RequestId, Version};
use meshway_protocol::{execution::*, node::NodeSessionResponse};

fn rid() -> RequestId {
    RequestId::new(random_uuid().unwrap()).unwrap()
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

fn instance(lease: &InstanceLease) -> InstanceRequest {
    InstanceRequest {
        instance_id: lease.instance_id,
        generation: lease.generation,
    }
}

fn demand(lease: &InstanceLease, target: NodeId) -> DemandRequest {
    DemandRequest {
        instance: instance(lease),
        request_id: rid(),
        target: BusinessTarget::Mesh { node: target },
        protocol: BusinessProtocol::Tcp,
        port: Some(443.try_into().unwrap()),
        profile: "mesh".into(),
        duration_seconds: 30,
    }
}

fn renewal(create: &DemandRequest, expires_at_ms: u64) -> RenewDemandRequest {
    RenewDemandRequest {
        instance: create.instance.clone(),
        request_id: create.request_id,
        expires_at_ms,
    }
}

async fn configuration(
    app: &App,
    token: &NodeSessionResponse,
    lease: &InstanceLease,
) -> NodeConfigurationResponse {
    app.execution()
        .configuration(
            token.token.expose(),
            NodeConfigurationRequest {
                instance: instance(lease),
                base: None,
            },
        )
        .await
        .unwrap()
}

#[derive(Debug, PartialEq)]
struct DemandRecord {
    payload: Vec<u8>,
    digest: Vec<u8>,
    expiry: i64,
    cancelled: Option<i64>,
}

#[derive(Debug, PartialEq)]
struct ExecutionRecord {
    node: Vec<u8>,
    target: Option<Vec<u8>>,
    instance_expiry: Option<i64>,
    execution_expiry: Option<i64>,
}

#[derive(Debug, PartialEq)]
struct Snapshot {
    publications: i64,
    demands: Vec<DemandRecord>,
    executions: Vec<ExecutionRecord>,
}

async fn snapshot(app: &App, network: NetworkId) -> Snapshot {
    app.executor.handle().execute(move |db| {
        let network = network.as_uuid().as_bytes().as_slice();
        let publications = db.query_row("SELECT count(*) FROM execution_publications WHERE network_id=?1", [network], |row| row.get(0)).unwrap();
        let demands = db.prepare("SELECT payload,request_digest,expires_at,cancelled_at FROM execution_demands WHERE network_id=?1 ORDER BY id").unwrap()
            .query_map([network], |row| Ok(DemandRecord { payload: row.get(0)?, digest: row.get(1)?, expiry: row.get(2)?, cancelled: row.get(3)? })).unwrap()
            .collect::<Result<Vec<_>, _>>().unwrap();
        let executions = db.prepare("SELECT node_id,target_publication_id,instance_expires_at,execution_expires_at FROM node_execution WHERE network_id=?1 ORDER BY node_id").unwrap()
            .query_map([network], |row| Ok(ExecutionRecord { node: row.get(0)?, target: row.get(1)?, instance_expiry: row.get(2)?, execution_expiry: row.get(3)? })).unwrap()
            .collect::<Result<Vec<_>, _>>().unwrap();
        Snapshot { publications, demands, executions }
    }).await.unwrap()
}

mod access;

mod authorization;

mod renewal;
