use super::*;

pub(super) fn configuration(
    facts: &RoutingFacts,
    report: &meshway_policy::CompileReport,
    id: &str,
) -> Result<ConfigurationSnapshot, Error> {
    let mut nodes = facts.nodes.clone();
    let index = nodes
        .iter()
        .position(|node| node.id == id)
        .ok_or(Error::Forbidden)?;
    let node = nodes.remove(index);
    let valid = report.is_valid() && !facts.mesh_v3;
    let payload = NodeConfiguration {
        schema_version: CONFIGURATION_SCHEMA_VERSION,
        revision: facts.revision,
        network: facts.network.clone(),
        node,
        peers: nodes,
        routing_policy: if valid {
            report.normalized_policy.clone()
        } else {
            RoutingPolicy::default()
        },
        policy_valid: valid,
        diagnostics: if facts.mesh_v3 {
            vec![PolicyDiagnostic { rule_id: None, field: "source_mode".into(), message: "Network uses Mesh v3 execution; this legacy view is inventory-only and authorizes no packets".into() }]
        } else {
            configuration::bounded_diagnostics(report.diagnostics.clone())
        },
    };
    let digest = configuration_digest(&payload)
        .map_err(|_| Error::Configuration("cannot encode configuration"))?;
    Ok(ConfigurationSnapshot { digest, payload })
}

pub(super) fn identities(
    id: &str,
    connections: &NodeConnections,
    access: &MeshPathAuthorization,
    keys: &BTreeMap<String, String>,
) -> Result<Vec<ConnectionPeer>, Error> {
    let ids: BTreeSet<_> = connections
        .peers
        .iter()
        .map(|peer| peer.node_id.as_str())
        .chain(
            access
                .grants
                .iter()
                .flat_map(|grant| grant.path.iter().map(String::as_str)),
        )
        .filter(|peer| *peer != id)
        .collect();
    if ids.len() > MAX_DATAPLANE_IDENTITIES {
        return Err(Error::Capacity(
            "dataplane exceeds 64 union peer identities",
        ));
    }
    ids.into_iter()
        .map(|id| {
            let key = keys
                .get(id)
                .filter(|key| valid_connection_public_key(key))
                .ok_or(Error::Configuration(
                    "dataplane peer has no canonical immutable binding",
                ))?;
            Ok(ConnectionPeer {
                node_id: id.to_owned(),
                public_key: key.clone(),
            })
        })
        .collect()
}
