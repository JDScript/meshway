//! Server-side checks that every actor applied a revision and that the
//! measured carriers are the expected ones.
use super::*;

pub(super) async fn measured_links(
    cli: &Cli,
    client: &Client,
    network: NetworkId,
    token: &str,
    nodes: &BTreeMap<Role, NodeId>,
    fallback: bool,
    stage: &str,
) -> Result<()> {
    tokio::time::timeout(STARTUP, async {
        for role in Role::ALL {
            loop {
                let view = client
                    .get_node_state(
                        &GetNodeStatePath {
                            network_id: network,
                            node_id: nodes[&role],
                        },
                        token,
                    )
                    .await?;
                let state = view
                    .nodes
                    .get(&nodes[&role])
                    .ok_or("missing traffic NodeState")?;
                tokio::fs::write(
                    cli.evidence
                        .join(format!("state-{stage}-attempt-{}.json", role.name())),
                    serde_json::to_vec_pretty(&view)?,
                )
                .await?;
                let Some(VisibleReport::Full { report }) = &state.report else {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    continue;
                };
                let peers: &[Role] = match role {
                    Role::A | Role::B => &[Role::R],
                    Role::R => &[Role::A, Role::B],
                };
                let complete = state.freshness == Freshness::Current
                    && report.configuration.applied.is_some()
                    && report.configuration.applied == report.configuration.saved
                    && peers.iter().all(|peer| {
                        report.links.get(&nodes[peer]).is_some_and(|links| {
                            links.iter().any(|link| {
                                link.phase == meshway_model::state::LinkPhase::Authenticated
                                    && link.transport
                                        == expected_carrier(cli, role, *peer, fallback).as_str()
                                    && link.business_dependencies > 0
                            }) && links.iter().all(|link| {
                                link.phase != meshway_model::state::LinkPhase::Authenticated
                                    || link.business_dependencies == 0
                                    || link.transport
                                        == expected_carrier(cli, role, *peer, fallback).as_str()
                            })
                        })
                    });
                if report.links.iter().any(|(peer, links)| {
                    !peers.iter().any(|role| nodes[role] == *peer)
                        && links.iter().any(|link| {
                            link.phase == meshway_model::state::LinkPhase::Authenticated
                        })
                }) {
                    return Err("unexpected authenticated edge bypasses mandatory relay".into());
                }
                if !complete {
                    // Installation, live adjacency and reported facts have independent
                    // lifetimes. Wait for a fresh real report after physical reattachment;
                    // rereading a report never creates or extends the observation.
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    continue;
                }
                if cli.traversal == crate::TraversalMode::Nat {
                    crate::traversal::verify_state(cli, role, report).await?;
                }
                tokio::fs::write(
                    cli.evidence
                        .join(format!("state-{stage}-{}.json", role.name())),
                    serde_json::to_vec_pretty(&view)?,
                )
                .await?;
                break;
            }
        }
        Ok(())
    })
    .await?
}

fn expected_carrier(
    cli: &Cli,
    role: Role,
    peer: Role,
    fallback: bool,
) -> meshway_model::TransportProtocol {
    if cli.traversal_fallback
        && fallback
        && matches!((role, peer), (Role::A, Role::R) | (Role::R, Role::A))
    {
        meshway_model::TransportProtocol::NativeTcp
    } else {
        cli.transport.protocol()
    }
}

pub(super) async fn applied(
    cli: &Cli,
    client: &Client,
    network: NetworkId,
    token: &str,
    nodes: &BTreeMap<Role, NodeId>,
    revision: Revision,
    observation: (&str, Option<&BTreeMap<Role, meshway_protocol::ipc::Status>>),
) -> Result<()> {
    let (stage, expected) = observation;
    tokio::time::timeout(STARTUP, async {
        for role in Role::ALL {
            loop {
                let view = client
                    .get_node_state(
                        &GetNodeStatePath {
                            network_id: network,
                            node_id: nodes[&role],
                        },
                        token,
                    )
                    .await?;
                let state = view
                    .nodes
                    .get(&nodes[&role])
                    .ok_or("missing Server NodeState")?;
                tokio::fs::write(
                    cli.evidence
                        .join(format!("state-{stage}-attempt-{}.json", role.name())),
                    serde_json::to_vec_pretty(&view)?,
                )
                .await?;
                if let Some(VisibleReport::Full { report }) = &state.report
                    && report
                        .configuration
                        .saved
                        .as_ref()
                        .is_some_and(|saved| saved.revision == revision)
                    && report
                        .configuration
                        .applied
                        .as_ref()
                        .is_some_and(|applied| applied.revision == revision)
                    && state.freshness == Freshness::Current
                    && expected.is_none_or(|expected| {
                        expected.get(&role).is_some_and(|status| {
                            state.instance_id
                                == status
                                    .instance
                                    .as_ref()
                                    .map(|instance| instance.instance_id)
                                && report.configuration.applied == report.configuration.saved
                        })
                    })
                {
                    tokio::fs::write(
                        cli.evidence
                            .join(format!("state-{stage}-{}.json", role.name())),
                        serde_json::to_vec_pretty(&view)?,
                    )
                    .await?;
                    break;
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
        Ok::<(), Error>(())
    })
    .await?
}
