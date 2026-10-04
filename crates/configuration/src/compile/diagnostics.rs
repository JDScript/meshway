use super::*;

pub(super) fn install(
    input: &CompilationInput<'_>,
    graph: &paths::Graph,
    diagnostics: &BTreeMap<meshway_model::ObservationId, DiagnosticDemand>,
    nodes: &mut BTreeMap<NodeId, NodeConfiguration>,
) -> Result<(), CompileError> {
    if diagnostics.len() > 128 {
        return Err(CompileError::Budget);
    }
    for (id, diagnostic) in diagnostics {
        if *id != diagnostic.id {
            return Err(CompileError::InvalidInput);
        }
        if diagnostic.expires_at_ms <= input.now_ms {
            continue;
        }
        let (protocol, port) = match diagnostic.service {
            ObservationServiceScope::Tcp { port } => (BusinessProtocol::Tcp, Some(port)),
            ObservationServiceScope::Udp { port } => (BusinessProtocol::Udp, Some(port)),
            ObservationServiceScope::IcmpEcho => (BusinessProtocol::IcmpEcho, None),
        };
        // Evaluate the explicitly requested real service with the exact business
        // compiler. This value is never installed as a business demand/permit.
        let scope = Demand {
            id: RequestId::new(*id.as_uuid()).map_err(|_| CompileError::InvalidInput)?,
            source: diagnostic.source,
            target: BusinessTarget::Mesh {
                node: diagnostic.target,
            },
            protocol,
            port,
            profile: selected_profile(input, diagnostic.source)?,
            expires_at_ms: diagnostic.expires_at_ms,
        };
        let DemandDecision::Permitted { permit } = evaluate(input, graph, &scope)? else {
            continue;
        };
        let paths: Vec<_> = permit
            .paths
            .into_iter()
            .filter(|p| {
                diagnostic
                    .path
                    .as_ref()
                    .is_none_or(|requested| requested == &p.nodes)
            })
            .collect();
        if paths.is_empty() {
            continue;
        }
        let permit = DiagnosticPermit {
            observation_id: *id,
            source: diagnostic.source,
            instance_id: diagnostic.instance_id,
            target: diagnostic.target,
            service: diagnostic.service.clone(),
            paths,
            replies: DiagnosticReplyPolicy::AuthenticatedExactReversePath,
            expires_at_ms: permit.expires_at_ms,
        };
        for path in &permit.paths {
            for id in &path.nodes {
                let node = nodes.get_mut(id).ok_or(CompileError::InvalidInput)?;
                node.diagnostics.insert(diagnostic.id, permit.clone());
                for peer in path.nodes.iter().filter(|peer| *peer != id) {
                    let settings = &input.configuration.nodes[peer];
                    let trusted = &input.inventory[peer];
                    node.peers.insert(
                        *peer,
                        PeerConfiguration {
                            identity: settings.identity.clone(),
                            addresses: settings.addresses.clone(),
                            control_key: trusted.control_key.clone(),
                            dataplane_key: trusted.dataplane_key.clone(),
                        },
                    );
                }
                if let Some(keys) = path.dial_candidates.get(id) {
                    for key in keys {
                        node.dialers
                            .insert(key.clone(), graph.candidates.borrow()[id][key].clone());
                    }
                }
            }
        }
    }
    Ok(())
}
