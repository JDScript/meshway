use super::{authorize_node, current_configuration, now, subjects};
use crate::{error::Error, secrets::Sealer};
use meshway_configuration::{
    mesh_access_allows_any, node_dns_name, normalize_dns_name, valid_node_hostname,
};
use meshway_model::{InstanceId, RequestId};
use meshway_protocol::node::{NodeDnsQueryRequest, NodeDnsQueryResponse, NodeDnsRecord};
use rusqlite::{Transaction, params};

pub(crate) fn query_dns(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    boot: InstanceId,
    token: [u8; 32],
    request: NodeDnsQueryRequest,
    request_id: RequestId,
) -> Result<NodeDnsQueryResponse, Error> {
    let now = now()?;
    let source = authorize_node(tx, sealer, boot, token, now)?;
    let name = normalize_dns_name(&request.name).map_err(|_| Error::InvalidRequest)?;
    let (revision, configuration, bindings) = current_configuration(tx, sealer, source.network)?;
    let subjects = subjects(tx, source.network, &bindings)?;
    if !subjects.node_is_active(&configuration, source.node) {
        return Err(Error::InvalidCredentials);
    }
    let mut response = NodeDnsQueryResponse {
        request_id,
        network_id: source.network,
        source_node_id: source.node,
        configuration_revision: revision,
        as_of: chrono::DateTime::from_timestamp_millis(now)
            .ok_or(Error::Internal("discovery timestamp"))?,
        record: None,
    };
    if !configuration.dns.magic_dns {
        return Ok(response);
    }
    let suffix = format!(".{}", configuration.dns.domain);
    let Some(hostname) = name
        .strip_suffix(&suffix)
        .filter(|host| valid_node_hostname(host))
    else {
        return Ok(response);
    };
    if let Some((node, settings)) = configuration
        .nodes
        .iter()
        .find(|(_, node)| node.hostname == hostname)
    {
        let fqdn = node_dns_name(&settings.hostname, &configuration.dns.domain)
            .map_err(|_| Error::decode("configuration DNS name"))?;
        if fqdn != name {
            return Err(Error::decode("configuration DNS name"));
        }
        // Registry lifecycle is distinct from execution key/instance availability.
        let registered: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM nodes WHERE network_id=?1 AND id=?2 AND credentials_revoked_at IS NULL)",
            params![source.network.as_uuid().as_bytes().as_slice(), node.as_uuid().as_bytes().as_slice()],
            |row| row.get(0),
        )?;
        if registered
            && subjects.node_is_active(&configuration, *node)
            && (*node == source.node
                || mesh_access_allows_any(&configuration, &subjects, source.node, *node))
        {
            response.record = Some(NodeDnsRecord {
                node_id: *node,
                name: fqdn,
                addresses: settings.addresses.clone(),
            });
        }
    }
    Ok(response)
}
