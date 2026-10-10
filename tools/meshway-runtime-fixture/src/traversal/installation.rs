//! Strict decoding of actual channel-installation provenance. A later
//! snapshot, candidate or debug label cannot stand in for the installation.
use crate::{
    Cli, Result, actor,
    signals::{Role, Signal, Signals},
};
use meshway_configuration::execution::Carrier;
use meshway_model::{BoundNodeKey, NetmapRef, NetworkId, NodeId, observations::ProbeChannel};
use meshway_protocol::InstanceRequest;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Installation {
    pub node: NodeId,
    pub network: NetworkId,
    pub instance: InstanceRequest,
    pub netmap: NetmapRef,
    pub expires_at_ms: u64,
    pub observed_at_ms: u64,
    pub control_key: BoundNodeKey,
    pub dataplane_key: BoundNodeKey,
    pub peer_control_key: BoundNodeKey,
    pub peer_dataplane_key: BoundNodeKey,
    pub carrier: Carrier,
    pub channel: ProbeChannel,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Installations {
    pub role: Role,
    pub status: Box<meshway_ipc::Status>,
    pub installed: Vec<Installation>,
}

#[derive(Deserialize)]
struct Fields {
    install_schema: u64,
    install_json: String,
}

pub(super) fn decode(event: &serde_json::Value) -> Result<Option<Installation>> {
    let Some(fields) = event.get("fields") else {
        return Ok(None);
    };
    if event.get("target").and_then(serde_json::Value::as_str)
        != Some("meshway_connectivity::manager")
        || fields.get("message").and_then(serde_json::Value::as_str)
            != Some("Connectivity channel transition")
        || fields.get("operation").and_then(serde_json::Value::as_str) != Some("install")
        || fields.get("cause").and_then(serde_json::Value::as_str) != Some("authenticated_ready")
    {
        return Ok(None);
    }
    let fields: Fields = serde_json::from_value(fields.clone())?;
    if fields.install_schema != 2 {
        return Err("unsupported installation evidence schema".into());
    }
    Ok(Some(serde_json::from_str(&fields.install_json)?))
}

pub async fn publish_installations(cli: &Cli, signals: &Signals, role: Role) -> Result<()> {
    let status = actor::command(cli, "status").await?;
    let identity = status
        .identity
        .as_ref()
        .ok_or("installation evidence has no actor identity")?;
    let instance = status
        .instance
        .as_ref()
        .ok_or("installation evidence has no current instance")?;
    let mut installed = Vec::new();
    // log_events bounds input to 16 MiB and ignores a partial final line.
    for event in actor::log_events(cli).await? {
        if let Some(installation) = decode(&event)?
            && installation.node == identity.node_id
            && installation.network == identity.network_id
            && installation.instance.instance_id == instance.instance_id
            && installation.instance.generation == instance.generation
        {
            installed.push(installation);
            if installed.len() > 256 {
                return Err("installation evidence record bound exceeded".into());
            }
        }
    }
    if installed.is_empty() {
        return Err("no actual authenticated channel installation evidence".into());
    }
    signals
        .write(
            &format!("traversal-installations-{}", role.name()),
            &Signal::Installations {
                evidence: Box::new(Installations {
                    role,
                    status: Box::new(status),
                    installed,
                }),
            },
        )
        .await
}
