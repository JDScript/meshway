//! Live interface replacement is driven by the isolated Docker owner, while
//! this actor checks the real Server State and the existing business path.
use super::*;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct AddressesChanged {
    old: std::net::IpAddr,
    new: std::net::IpAddr,
}

pub(super) async fn run(
    cli: &Cli,
    signals: &Signals,
    client: &Client,
    network: NetworkId,
    token: &str,
    nodes: &BTreeMap<Role, NodeId>,
) -> Result<()> {
    if !cli.reported_endpoints {
        return Ok(());
    }
    for role in Role::ALL {
        signals
            .ready(&format!("interface-ready-{}", role.name()))
            .await?;
    }
    let addresses: AddressesChanged = serde_json::from_slice(
        &tokio::fs::read(cli.signals.join("interface-addresses.json")).await?,
    )?;
    for (stage, current, removed) in [
        ("changed", addresses.new, addresses.old),
        ("restored", addresses.old, addresses.new),
    ] {
        signals
            .write(&format!("interface-request-{stage}"), &Signal::Ready)
            .await?;
        signals.ready(&format!("interface-{stage}")).await?;
        tokio::time::timeout(Duration::from_secs(45), async {
            loop {
                let view = client
                    .get_node_state(
                        &GetNodeStatePath {
                            network_id: network,
                            node_id: nodes[&Role::R],
                        },
                        token,
                    )
                    .await?;
                let state = view
                    .nodes
                    .get(&nodes[&Role::R])
                    .ok_or("missing relay State")?;
                if state.freshness == Freshness::Current
                    && let Some(VisibleReport::Full { report }) = &state.report
                    && report.configuration.applied.is_some()
                {
                    let endpoints: Vec<_> = report
                        .sockets
                        .values()
                        .filter_map(|socket| socket.local_endpoints.as_ref())
                        .flatten()
                        .map(|endpoint| endpoint.address.ip())
                        .collect();
                    if endpoints.contains(&current) && !endpoints.contains(&removed) {
                        tokio::fs::write(
                            cli.evidence.join(format!("interface-{stage}-state.json")),
                            serde_json::to_vec_pretty(&view)?,
                        )
                        .await?;
                        break;
                    }
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            Ok::<_, Error>(())
        })
        .await??;
        signals
            .write(&format!("interface-state-{stage}"), &Signal::Ready)
            .await?;
        signals.ready(&format!("interface-traffic-{stage}")).await?;
        measured_links(
            cli,
            client,
            network,
            token,
            nodes,
            false,
            &format!("interface-{stage}"),
        )
        .await?;
        for role in [Role::A, Role::B] {
            let view = client
                .get_node_state(
                    &GetNodeStatePath {
                        network_id: network,
                        node_id: nodes[&role],
                    },
                    token,
                )
                .await?;
            let Some(VisibleReport::Full { report }) = &view
                .nodes
                .get(&nodes[&role])
                .ok_or("missing dialing peer State")?
                .report
            else {
                return Err("missing dialing peer report".into());
            };
            if !report.links.get(&nodes[&Role::R]).is_some_and(|links| {
                links.iter().any(|link| {
                    link.phase == meshway_model::state::LinkPhase::Authenticated
                        && link.initiator_node_id == nodes[&role]
                        && link.acquisition
                            == Some(meshway_model::observations::LinkAcquisition::LocalEndpoint)
                })
            }) {
                return Err("interface recovery did not redial the relay's local candidate".into());
            }
        }
    }
    signals.write("interface-complete", &Signal::Ready).await?;
    Ok(())
}
