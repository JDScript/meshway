//! Closed decoding of the successful-install log's typed fields. Debug legacy
//! fields are deliberately not inputs to channel identity or provenance.
use crate::{
    Cli, Result, actor,
    signals::{Role, Signal, Signals},
};
use meshway_configuration::execution::Carrier;
use meshway_model::{
    BoundNodeKey, ConfigurationRef, Digest, InstanceId, KeyAlgorithm, KeyId, NetworkId, NodeId,
    NodePublicKey, PublicationId, Revision, Version,
    observations::{EndpointKind, LinkAcquisition, ProbeChannel},
};
use meshway_protocol::InstanceRequest;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Installation {
    pub node: NodeId,
    pub network: NetworkId,
    pub instance: InstanceRequest,
    pub configuration: ConfigurationRef,
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
    /// Identity/instance read from the same actor daemon, not channel evidence.
    pub status: Box<meshway_ipc::Status>,
    pub installed: Vec<Installation>,
}

#[derive(Deserialize)]
struct Fields {
    install_schema: u64,
    install_observed_at_ms: u64,
    install_node: NodeId,
    install_network: NetworkId,
    install_instance: InstanceId,
    install_instance_generation: Version,
    install_revision: Revision,
    publication: PublicationId,
    install_digest: Digest,
    execution_expires_at_ms: u64,
    install_link: Digest,
    install_socket: Digest,
    install_protocol: meshway_transport_spec::TransportProtocol,
    install_acquisition: Acquisition,
    install_attempt_id: String,
    install_endpoint_source: String,
    install_control_id: KeyId,
    install_control_generation: Version,
    install_control_bytes: Digest,
    install_dataplane_id: KeyId,
    install_dataplane_generation: Version,
    install_dataplane_bytes: Digest,
    install_peer_control_id: KeyId,
    install_peer_control_generation: Version,
    install_peer_control_bytes: Digest,
    install_peer_dataplane_id: KeyId,
    install_peer_dataplane_generation: Version,
    install_peer_dataplane_bytes: Digest,
    peer: NodeId,
    dialer: NodeId,
    listener: NodeId,
    listener_key: String,
    local: meshway_model::SocketAddress,
    remote: meshway_model::SocketAddress,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
enum Acquisition {
    PublishedEndpoint,
    AcceptedInbound,
    CoordinatedUdp,
}

fn key(id: KeyId, generation: Version, bytes: Digest, algorithm: KeyAlgorithm) -> BoundNodeKey {
    // Schema 1 records registered Control Ed25519 and dataplane X25519 keys;
    // these purposes/algorithms were checked before the install. All changing
    // binding components (ID, generation and public bytes) come from its scope.
    BoundNodeKey {
        id,
        generation,
        public_key: NodePublicKey { algorithm, bytes },
    }
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
    if fields.install_schema != 1 {
        return Err("unsupported installation evidence schema".into());
    }
    let acquisition = match fields.install_acquisition {
        Acquisition::CoordinatedUdp => LinkAcquisition::CoordinatedUdp {
            attempt_id: fields.install_attempt_id.parse()?,
            endpoint_source: match fields.install_endpoint_source.as_str() {
                "configured" => EndpointKind::Configured,
                "interface" => EndpointKind::Interface,
                "stun" => EndpointKind::Stun,
                _ => return Err("unknown actual endpoint source".into()),
            },
        },
        kind => {
            if !fields.install_attempt_id.is_empty() || !fields.install_endpoint_source.is_empty() {
                return Err("non-coordinated install claimed traversal provenance".into());
            }
            match kind {
                Acquisition::PublishedEndpoint => LinkAcquisition::PublishedEndpoint,
                Acquisition::AcceptedInbound => LinkAcquisition::AcceptedInbound,
                Acquisition::CoordinatedUdp => unreachable!(),
            }
        }
    };
    Ok(Some(Installation {
        node: fields.install_node,
        network: fields.install_network,
        instance: InstanceRequest {
            instance_id: fields.install_instance,
            generation: fields.install_instance_generation,
        },
        configuration: ConfigurationRef {
            revision: fields.install_revision,
            execution_publication: fields.publication,
            digest: fields.install_digest,
        },
        expires_at_ms: fields.execution_expires_at_ms,
        observed_at_ms: fields.install_observed_at_ms,
        control_key: key(
            fields.install_control_id,
            fields.install_control_generation,
            fields.install_control_bytes,
            KeyAlgorithm::Ed25519,
        ),
        dataplane_key: key(
            fields.install_dataplane_id,
            fields.install_dataplane_generation,
            fields.install_dataplane_bytes,
            KeyAlgorithm::X25519,
        ),
        peer_control_key: key(
            fields.install_peer_control_id,
            fields.install_peer_control_generation,
            fields.install_peer_control_bytes,
            KeyAlgorithm::Ed25519,
        ),
        peer_dataplane_key: key(
            fields.install_peer_dataplane_id,
            fields.install_peer_dataplane_generation,
            fields.install_peer_dataplane_bytes,
            KeyAlgorithm::X25519,
        ),
        carrier: Carrier {
            dialer: fields.dialer,
            listener: fields.listener,
            listener_key: fields.listener_key,
            protocol: fields.install_protocol,
        },
        channel: ProbeChannel {
            link_id: fields.install_link,
            socket_id: fields.install_socket,
            peer_node_id: fields.peer,
            transport: fields.install_protocol,
            local_address: fields.local,
            remote_address: fields.remote,
            acquisition,
        },
    }))
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
    // log_events bounds the complete input to 16 MiB and ignores a partial final
    // line. Records from an earlier daemon instance cannot claim this owner.
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
    let evidence = Installations {
        role,
        status: Box::new(status),
        installed,
    };
    signals
        .write(
            &format!("traversal-installations-{}", role.name()),
            &Signal::Installations {
                evidence: Box::new(evidence),
            },
        )
        .await
}
