use super::*;

fn authorization_id(path: &[String]) -> String {
    local::digest(&format!("effective:{}", path.join(":")))
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
thread_local! {
    pub(super) static COMPILATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

pub(super) fn prepare<'a>(
    policy: &RoutingPolicy,
    network: &MeshNetwork,
    nodes: &'a [MeshNode],
) -> meshway_policy::PreparedRouting<'a> {
    #[cfg(test)]
    COMPILATIONS.with(|count| count.set(count.get() + 1));
    meshway_policy::prepare_routing(policy, network, nodes)
}

pub(super) fn authorizations(
    facts: &Facts,
    prepared: &meshway_policy::PreparedRouting<'_>,
    adjacency: &mut Adjacency<'_>,
    mut route_evaluated: impl FnMut(),
) -> Result<(bool, BTreeMap<String, MeshPathAuthorization>), Error> {
    let nodes = &facts.routing.nodes;
    let mut result: BTreeMap<_, _> = nodes
        .iter()
        .map(|node| (node.id.clone(), MeshPathAuthorization::default()))
        .collect();
    let access = &facts.access;
    if facts.routing.mesh_policy_active {
        return Ok((false, result));
    }
    if !meshway_policy::validate_access_policy(access, nodes, &facts.catalog).is_empty() {
        return Ok((false, result));
    }
    if access.default_action == AccessAction::Reject
        && access
            .rules
            .iter()
            .all(|rule| rule.action == AccessAction::Reject)
    {
        return Ok((true, result));
    }
    let eligible: Vec<_> = nodes
        .iter()
        .filter(|node| {
            facts
                .keys
                .get(&node.id)
                .is_some_and(|key| valid_connection_public_key(key))
        })
        .collect();
    if eligible.len() < 2 || !prepared.report().is_valid() {
        return Ok((true, result));
    }
    let eligible_ids: BTreeSet<_> = eligible.iter().map(|node| node.id.as_str()).collect();
    let mut identity_unions: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for source in &eligible {
        for destination in &eligible {
            if source.id == destination.id
                || meshway_policy::evaluate_access(
                    access,
                    &source.id,
                    &destination.id,
                    &facts.assignments,
                )
                .action
                    != FlowAuthorizationAction::Allow
            {
                continue;
            }
            route_evaluated();
            let Some(path) = prepared.compile_mesh_endpoint(&source.id, &destination.id) else {
                continue;
            };
            if path.first() != Some(&source.id)
                || path.last() != Some(&destination.id)
                || path.iter().any(|id| !eligible_ids.contains(id.as_str()))
            {
                continue;
            }
            if path.len() > MAX_MESH_ACCESS_PATH_NODES {
                return Err(Error::Capacity(
                    "authorized route exceeds the 8-node exact-path limit",
                ));
            }
            let grant = MeshAccessGrant {
                id: authorization_id(&path),
                path,
            };
            for participant in &grant.path {
                let authorization = result.get(participant).ok_or(Error::Configuration(
                    "routing returned an inactive path node",
                ))?;
                if authorization.grants.len() == MAX_MESH_ACCESS_GRANTS {
                    return Err(Error::Capacity(
                        "effective access exceeds 128 exact paths for a node",
                    ));
                }
                if !identity_unions.contains_key(participant) {
                    let projected = adjacency.project(participant)?;
                    identity_unions.insert(
                        participant.clone(),
                        projected
                            .payload
                            .peers
                            .iter()
                            .map(|peer| peer.node_id.clone())
                            .collect(),
                    );
                }
                let union = identity_unions
                    .get_mut(participant)
                    .expect("inserted participant identity union");
                union.extend(grant.path.iter().filter(|id| *id != participant).cloned());
                if union.len() > MAX_DATAPLANE_IDENTITIES {
                    return Err(Error::Capacity(
                        "dataplane exceeds 64 union peer identities",
                    ));
                }
            }
            for participant in &grant.path {
                result
                    .get_mut(participant)
                    .expect("validated path participant")
                    .grants
                    .push(grant.clone());
            }
        }
    }
    Ok((true, result))
}
