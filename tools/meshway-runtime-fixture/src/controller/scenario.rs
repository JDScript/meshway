//! Controller side of the netmap lifecycle rows. Each runs after the shared
//! startup, initial diagnostics and persistent traffic, on the same real
//! Server, and leaves the actors to the shared offline `down` check.
use super::{applied, apply, source};
use crate::{
    Cli, IO, NetmapScenario, Result, STARTUP,
    recovery::Availability,
    request,
    signals::{Role, Signal, Signals},
};
use meshway_control_client::{Client, DeleteNodePath};
use meshway_model::{NetworkId, NodeId, Revision};
use std::{collections::BTreeMap, time::Duration};

/// A configuration change must reach the affected data plane within this.
pub(super) const PROPAGATION: Duration = Duration::from_secs(5);

pub(super) struct Context<'a> {
    pub cli: &'a Cli,
    pub signals: &'a Signals,
    pub client: &'a Client,
    pub network: NetworkId,
    pub token: &'a str,
    pub nodes: &'a BTreeMap<Role, NodeId>,
    pub revision: Revision,
    pub availability: &'a Availability,
    pub server: &'a super::server::Server<'a>,
}

pub(super) async fn run(scenario: NetmapScenario, context: Context<'_>) -> Result<()> {
    tracing::info!(
        phase = "netmap-scenario",
        scenario = scenario.name(),
        "shared startup complete"
    );
    match scenario {
        NetmapScenario::Offline => outage(&context, "offline", window(context.cli)).await,
        NetmapScenario::Expiry => outage(&context, "expiry", Duration::ZERO).await,
        NetmapScenario::Restart => outage(&context, "restart", Duration::ZERO).await,
        NetmapScenario::Revoke => revoke(&context).await,
        NetmapScenario::Relay => relay(&context).await,
        NetmapScenario::Reconnect => reconnect(&context).await,
    }
}

/// Server outage required by the offline row: 3 minutes (three 60 s lease
/// validities, so renewals and the idle adjacency close all fall inside it),
/// or 60 s in the non-acceptance short mode.
pub(super) fn window(cli: &Cli) -> Duration {
    Duration::from_secs(if cli.short { 60 } else { 180 })
}

/// Close Node HTTP, keep it closed for `minimum` and until every actor has
/// checked its offline behaviour, then reopen and require recovery to the
/// same revision without a second `up`.
async fn outage(context: &Context<'_>, name: &str, minimum: Duration) -> Result<()> {
    let Context {
        cli,
        signals,
        availability,
        ..
    } = context;
    let closed_at_ms = availability.close().await?;
    signals
        .write(
            "scenario-closed",
            &Signal::HttpAvailability {
                available: false,
                observed_at_ms: closed_at_ms,
            },
        )
        .await?;
    tracing::info!(
        phase = "scenario-outage",
        scenario = name,
        closed_at_ms,
        "Node HTTP closed"
    );
    tokio::time::sleep(minimum).await;
    signals.write("scenario-window-end", &Signal::Ready).await?;
    let budget = cli.netmap_validity.as_std() + window(cli) + Duration::from_secs(240);
    for role in Role::ALL {
        match signals
            .wait_for(&format!("scenario-offline-{}", role.name()), budget)
            .await?
        {
            Signal::Observed {
                role: claimed,
                at_ms,
            } if claimed == role && at_ms > closed_at_ms => {}
            _ => return Err("invalid offline scenario evidence".into()),
        }
    }
    let (reopened_at_ms, rejected) = availability.open()?;
    tokio::fs::write(
        cli.evidence.join(format!("scenario-{name}-outage.json")),
        serde_json::to_vec_pretty(&crate::recovery::Window {
            closed_at_ms,
            reopened_at_ms,
            rejected_node_requests: rejected,
            response_status: 503,
        })?,
    )
    .await?;
    signals
        .write(
            "scenario-open",
            &Signal::HttpAvailability {
                available: true,
                observed_at_ms: reopened_at_ms,
            },
        )
        .await?;
    recovered(context, context.revision, "recovered").await
}

async fn recovered(context: &Context<'_>, revision: Revision, stage: &str) -> Result<()> {
    for role in Role::ALL {
        match context
            .signals
            .wait_for(
                &format!("scenario-{stage}-{}", role.name()),
                STARTUP + Duration::from_secs(90),
            )
            .await?
        {
            Signal::Observed { role: claimed, .. } if claimed == role => {}
            _ => return Err("invalid scenario recovery evidence".into()),
        }
    }
    applied(
        context.cli,
        context.client,
        context.network,
        context.token,
        context.nodes,
        revision,
        (&format!("scenario-{stage}"), None),
    )
    .await
}

use crate::recovery::now_ms;

/// One committed change, announced to the actors with its commit time; the
/// affected actor reports when its data plane showed the effect.
async fn step(
    context: &Context<'_>,
    name: &str,
    revision: Revision,
    at_ms: u64,
    observer: Role,
) -> Result<u64> {
    context
        .signals
        .write(
            &format!("scenario-{name}"),
            &Signal::Step { revision, at_ms },
        )
        .await?;
    let Signal::Observed { role, at_ms: seen } = context
        .signals
        .wait_for(&format!("scenario-{name}-observed"), STARTUP + IO)
        .await?
    else {
        return Err("invalid scenario observation".into());
    };
    let elapsed = seen
        .checked_sub(at_ms)
        .ok_or("observation precedes the change")?;
    tracing::info!(
        phase = "scenario-step",
        step = name,
        elapsed_ms = elapsed,
        "data plane effect observed"
    );
    if role != observer || Duration::from_millis(elapsed) > PROPAGATION {
        return Err(format!("{name}: data plane effect took {elapsed} ms").into());
    }
    Ok(elapsed)
}

/// Port revocation and restoration, B's key rotation, then B's membership
/// revocation; each new serial reaches A's data plane within 5 s.
async fn revoke(context: &Context<'_>) -> Result<()> {
    let Context {
        cli,
        signals,
        client,
        network,
        token,
        nodes,
        ..
    } = context;
    let original = source(client, *network, token).await?;
    let allowed = original.configuration.clone();
    let mut denied = allowed.clone();
    let rule = denied
        .access
        .first_mut()
        .ok_or("business access rule absent")?;
    let before = rule.ip.len();
    rule.ip.retain(|ip| ip != "tcp:42425");
    if rule.ip.len() + 1 != before {
        return Err("TCP port revocation did not remove exactly one service".into());
    }
    let mut timings = BTreeMap::new();
    let revision = apply(client, *network, token, original.revision, &denied).await?;
    let at = now_ms()?;
    timings.insert(
        "tcp-revoked",
        step(context, "tcp-revoked", revision, at, Role::A).await?,
    );
    applied(
        cli,
        client,
        *network,
        token,
        nodes,
        revision,
        ("scenario-tcp-revoked", None),
    )
    .await?;
    let revision = apply(client, *network, token, revision, &allowed).await?;
    let at = now_ms()?;
    timings.insert(
        "tcp-restored",
        step(context, "tcp-restored", revision, at, Role::A).await?,
    );
    applied(
        cli,
        client,
        *network,
        token,
        nodes,
        revision,
        ("scenario-tcp-restored", None),
    )
    .await?;
    // B rotates its own keys through its daemon; the Server's new serial
    // carries them to A and R, whose circuits re-handshake.
    signals.write("scenario-rotate", &Signal::Ready).await?;
    let Signal::Step { at_ms, .. } = signals.wait_for("scenario-rotated", STARTUP + IO).await?
    else {
        return Err("invalid key rotation evidence".into());
    };
    let Signal::Observed {
        role: Role::A,
        at_ms: seen,
    } = signals
        .wait_for("scenario-rotated-observed", STARTUP + IO)
        .await?
    else {
        return Err("invalid post-rotation traffic evidence".into());
    };
    timings.insert("key-rotated", seen.saturating_sub(at_ms));
    let current = source(client, *network, token).await?;
    client
        .delete_node(
            &DeleteNodePath {
                network_id: *network,
                node_id: nodes[&Role::B],
            },
            &meshway_protocol::DeleteNodeRequest {
                request_id: request()?,
                expected_revision: current.revision,
            },
            token,
        )
        .await?;
    let at = now_ms()?;
    let deleted = source(client, *network, token).await?.revision;
    timings.insert(
        "member-revoked",
        step(context, "member-revoked", deleted, at, Role::A).await?,
    );
    tokio::fs::write(
        cli.evidence.join("scenario-revoke-timings.json"),
        serde_json::to_vec_pretty(&timings)?,
    )
    .await?;
    for role in Role::ALL {
        match signals
            .wait_for(&format!("scenario-revoked-{}", role.name()), STARTUP + IO)
            .await?
        {
            Signal::Observed { role: claimed, .. } if claimed == role => {}
            _ => return Err("invalid membership revocation evidence".into()),
        }
    }
    Ok(())
}

/// A new flow must take the relay within this of the new content's commit.
const RELAY_SWITCH: Duration = Duration::from_secs(30);

/// Five Nodes. A's and R's NAT routers drop every packet to and from B, so
/// R (R1) keeps its channel with A but loses B. Two public relays enroll
/// now: the one with the lower Node ID is R3 (topology to both ends, a
/// resident edge only with B, so A has no channel with it); the other is R2
/// (resident edges with A and B). Then direct A–B is allowed (no `via`):
/// Policy lists [A, B] and every two-hop relay, A has no channel with B, and
/// new traffic and a CLI ping relay through R2, the relay with channels to
/// both ends, within [`RELAY_SWITCH`].
async fn relay(context: &Context<'_>) -> Result<()> {
    let Context {
        cli,
        signals,
        client,
        network,
        token,
        nodes,
        ..
    } = context;
    signals.write("relay-block", &Signal::Ready).await?;
    signals.ready("relay-blocked-router-a").await?;
    signals.ready("relay-blocked-router-r").await?;
    let extra = enroll_relays(context).await?;
    let (r3, r2) = (extra[0].1, extra[1].1);
    let underlay = crate::signals::underlay(cli).await?;
    let mut input = source(client, *network, token).await?.configuration;
    for (name, node) in &extra {
        let settings = input
            .nodes
            .get_mut(node)
            .ok_or("missing enrolled extra relay")?;
        let public = underlay.relays.get(*name).ok_or("missing relay underlay")?;
        settings.listeners = BTreeMap::from([(
            "udp".into(),
            meshway_transport::spec::Listener::NativeUdp(
                meshway_transport::spec::NativeUdpListener {
                    enabled: true,
                    remark: String::new(),
                    listen: "0.0.0.0".parse()?,
                    listen_port: 35000.try_into()?,
                    endpoints: vec![meshway_transport::spec::Endpoint {
                        host: public.to_string(),
                        port: 35000.try_into()?,
                    }],
                    bind_interface: String::new(),
                    socket: Default::default(),
                },
            ),
        )]);
    }
    for (a, b, keepalive) in [
        (nodes[&Role::A], r2, true),
        (r2, nodes[&Role::B], true),
        (nodes[&Role::A], r3, false),
        (r3, nodes[&Role::B], true),
    ] {
        input.topology.push(meshway_configuration::TopologyRule {
            remark: "nat/relay extra relay".into(),
            action: meshway_configuration::TopologyAction::Allow,
            a: format!("node:{a}"),
            b: format!("node:{b}"),
            transports: meshway_configuration::Input::Omitted,
            keepalive,
        });
    }
    let current = source(client, *network, token).await?.revision;
    let configured = apply(client, *network, token, current, &input).await?;
    signals
        .write(
            "relay-extra-config",
            &Signal::Revision {
                revision: configured,
            },
        )
        .await?;
    for name in crate::actor::relay::NAMES {
        signals.ready(&format!("relay-running-{name}")).await?;
    }
    let r1 = nodes[&Role::R];
    tokio::fs::write(
        cli.evidence.join("scenario-relay-roles.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "r1": r1, "r2": r2, "r3": r3,
            "r3BeforeR2": r3 < r2, "r1BeforeR2": r1 < r2,
        }))?,
    )
    .await?;
    // The channels this scenario ranks by: R2 with A and B, R3 with B
    // only, and R1 no longer with B, each in a fresh report of its own.
    settled(
        context,
        &[
            (r2, vec![nodes[&Role::A], nodes[&Role::B]], vec![]),
            (r3, vec![nodes[&Role::B]], vec![nodes[&Role::A]]),
            (r1, vec![nodes[&Role::A]], vec![nodes[&Role::B]]),
        ],
    )
    .await?;
    // Facts reach A with its next sync answer, which a fact change wakes.
    tokio::time::sleep(Duration::from_secs(3)).await;
    let original = source(client, *network, token).await?;
    let mut direct = original.configuration.clone();
    direct.routing.mesh.clear();
    direct.topology.push(meshway_configuration::TopologyRule {
        remark: "direct A-B, dropped by A's NAT".into(),
        action: meshway_configuration::TopologyAction::Allow,
        a: format!("node:{}", nodes[&Role::A]),
        b: format!("node:{}", nodes[&Role::B]),
        transports: meshway_configuration::Input::Omitted,
        keepalive: false,
    });
    signals
        .write(
            "scenario-relay-path",
            &Signal::Path {
                nodes: vec![nodes[&Role::A], r2, nodes[&Role::B]],
            },
        )
        .await?;
    let revision = apply(client, *network, token, original.revision, &direct).await?;
    let at_ms = now_ms()?;
    signals
        .write("scenario-relay", &Signal::Step { revision, at_ms })
        .await?;
    let Signal::Observed { role, at_ms: seen } = signals
        .wait_for("scenario-relay-observed", STARTUP + IO)
        .await?
    else {
        return Err("invalid relay observation".into());
    };
    let elapsed = seen
        .checked_sub(at_ms)
        .ok_or("observation precedes the change")?;
    tokio::fs::write(
        cli.evidence.join("scenario-relay-timing.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "committedAtMs": at_ms, "relayedAtMs": seen, "elapsedMs": elapsed
        }))?,
    )
    .await?;
    tracing::info!(
        phase = "scenario-step",
        step = "relay",
        elapsed_ms = elapsed,
        "relayed traffic observed"
    );
    if role != Role::A || Duration::from_millis(elapsed) > RELAY_SWITCH {
        return Err(format!("relay: first relayed reply took {elapsed} ms").into());
    }
    for role in Role::ALL {
        match signals
            .wait_for(
                &format!("scenario-relay-done-{}", role.name()),
                STARTUP + IO,
            )
            .await?
        {
            Signal::Observed { role: claimed, .. } if claimed == role => {}
            _ => return Err("invalid relay scenario evidence".into()),
        }
    }
    // A never acquired a channel with R3: the ranking did not need it.
    settled(
        context,
        &[(r3, vec![nodes[&Role::B]], vec![nodes[&Role::A]])],
    )
    .await?;
    let result = applied(
        cli,
        client,
        *network,
        token,
        nodes,
        revision,
        ("scenario-relay", None),
    )
    .await;
    signals.write("relay-extra-stop", &Signal::Ready).await?;
    result
}

/// Enroll the extra relays; returns their names and Node IDs, the lower ID
/// first (R3, then R2).
async fn enroll_relays(context: &Context<'_>) -> Result<Vec<(&'static str, NodeId)>> {
    let Context {
        signals,
        client,
        network,
        token,
        ..
    } = context;
    signals.write("relay-extra-enroll", &Signal::Ready).await?;
    let mut enrolled = Vec::new();
    for name in crate::actor::relay::NAMES {
        let Signal::Relay {
            name: claimed,
            status,
        } = signals.wait(&format!("relay-enrollment-{name}")).await?
        else {
            return Err("invalid extra relay enrollment status".into());
        };
        let enrollment = status
            .enrollment_request_id
            .filter(|_| claimed == name)
            .ok_or("extra relay has no pending enrollment")?;
        let request = client
            .get_device_authorization(
                &meshway_control_client::GetDeviceAuthorizationPath {
                    enrollment_request_id: enrollment,
                },
                token,
            )
            .await?;
        if request.request.hostname != format!("relay-{name}") {
            return Err("extra relay enrollment name mismatch".into());
        }
        client
            .approve_device_authorization(
                &meshway_control_client::ApproveDeviceAuthorizationPath {
                    enrollment_request_id: enrollment,
                },
                &meshway_protocol::ApproveEnrollmentRequest {
                    network_id: *network,
                },
                token,
            )
            .await?;
        let Signal::Relay {
            name: claimed,
            status,
        } = signals.wait(&format!("relay-enrolled-{name}")).await?
        else {
            return Err("invalid extra relay enrolled status".into());
        };
        let identity = status
            .identity
            .filter(|identity| claimed == name && identity.network_id == *network)
            .ok_or("extra relay identity mismatch")?;
        enrolled.push((name, identity.node_id));
    }
    enrolled.sort_by_key(|(_, node)| *node);
    Ok(enrolled)
}

/// Wait until each Node's own current report has an authenticated channel
/// with every Node of `with` and none with any of `without`.
async fn settled(
    context: &Context<'_>,
    expected: &[(NodeId, Vec<NodeId>, Vec<NodeId>)],
) -> Result<()> {
    let Context {
        cli,
        client,
        network,
        token,
        ..
    } = context;
    tokio::time::timeout(STARTUP * 2, async {
        for (node, with, without) in expected {
            loop {
                let view = client
                    .get_node_state(
                        &meshway_control_client::GetNodeStatePath {
                            network_id: *network,
                            node_id: *node,
                        },
                        token,
                    )
                    .await?;
                let channel = |report: &meshway_model::state::NodeState, peer: &NodeId| {
                    report.links.get(peer).is_some_and(|links| {
                        links.iter().any(|link| {
                            link.phase == meshway_model::state::LinkPhase::Authenticated
                        })
                    })
                };
                if let Some(state) = view.nodes.get(node)
                    && state.freshness == meshway_protocol::state::Freshness::Current
                    && let Some(meshway_protocol::state::VisibleReport::Full { report }) =
                        &state.report
                    && with.iter().all(|peer| channel(report, peer))
                    && !without.iter().any(|peer| channel(report, peer))
                {
                    tokio::fs::write(
                        cli.evidence
                            .join(format!("state-relay-settled-{node}.json")),
                        serde_json::to_vec_pretty(&view)?,
                    )
                    .await?;
                    break;
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }
        Ok::<(), crate::Error>(())
    })
    .await?
}

/// Each disruption of the punched A–R channel must be repaired by a new
/// direct channel within this.
const RECONNECT: Duration = Duration::from_secs(60);

/// A's router maps every new UDP flow to a random port and R's admits
/// inbound UDP: R is a public host, A is behind endpoint-dependent NAT, and
/// only coordinated traversal with source learning connects them. Four
/// disruptions in turn, each repaired within [`RECONNECT`]:
/// - `nat`: A–R UDP dropped until both ends retire the channel, then healed
///   with every A mapping forgotten (the NAT models switch at this break);
/// - `server`: the real Server restarts in place (every instance, State
///   report and fact ends with it), A and R take new instances, then the
///   same break and heal;
/// - `restart-r`, `restart-a`: one daemon restarts, the other keeps running.
async fn reconnect(context: &Context<'_>) -> Result<()> {
    let Context {
        cli,
        signals,
        client,
        network,
        token,
        nodes,
        revision,
        server,
        ..
    } = context;
    let mut timings = serde_json::Map::new();
    // Saved after every stage: a stage that never recovers keeps the others.
    let save = async |timings: &serde_json::Map<String, serde_json::Value>| -> Result<()> {
        tokio::fs::write(
            cli.evidence.join("scenario-reconnect-timings.json"),
            serde_json::to_vec_pretty(timings)?,
        )
        .await?;
        Ok(())
    };
    let observed = async |name: &str, budget: Duration| -> Result<u64> {
        match signals.wait_for(name, budget).await? {
            Signal::Observed { at_ms, .. } => Ok(at_ms),
            _ => Err(format!("invalid {name} evidence").into()),
        }
    };
    let routers = async |name: &str| -> Result<()> {
        for router in ["router-a", "router-r"] {
            signals.ready(&format!("{name}-{router}")).await?;
        }
        Ok(())
    };
    let step = async |name: &str| -> Result<u64> {
        let at_ms = now_ms()?;
        signals
            .write(
                name,
                &Signal::Step {
                    revision: *revision,
                    at_ms,
                },
            )
            .await?;
        Ok(at_ms)
    };
    // A's own status shows the channel gone, then (after the heal) a new one.
    let breaks = async |stage: &str| -> Result<serde_json::Value> {
        step(&format!("reconnect-break-{stage}")).await?;
        routers(&format!("reconnect-blocked-{stage}")).await?;
        let lost = observed(&format!("reconnect-lost-{stage}"), STARTUP + IO).await?;
        let healed = step(&format!("reconnect-heal-{stage}")).await?;
        routers(&format!("reconnect-healed-{stage}")).await?;
        let direct = observed(&format!("reconnect-direct-{stage}"), RECONNECT * 3).await?;
        Ok(serde_json::json!({ "lostAtMs": lost, "disruptedAtMs": healed, "directAtMs": direct }))
    };
    tracing::info!(phase = "reconnect", stage = "nat", "switching NAT models");
    timings.insert("nat".into(), breaks("nat").await?);
    save(&timings).await?;
    tracing::info!(phase = "reconnect", stage = "server", "restarting the Server");
    let stopped = now_ms()?;
    let restarted = server.restart().await?;
    step("reconnect-server").await?;
    let mut instances = serde_json::Map::new();
    for role in [Role::A, Role::R] {
        let at = observed(&format!("reconnect-instance-{}", role.name()), STARTUP + IO).await?;
        instances.insert(role.name().into(), at.into());
    }
    // State (and the facts derived from it) is volatile: the new owner knows
    // A and R again once they report under their new instances.
    let mut reported = serde_json::Map::new();
    for role in [Role::A, Role::R] {
        let at = tokio::time::timeout(STARTUP, async {
            loop {
                let view = client
                    .get_node_state(
                        &meshway_control_client::GetNodeStatePath {
                            network_id: *network,
                            node_id: nodes[&role],
                        },
                        token,
                    )
                    .await?;
                let current = view.nodes.get(&nodes[&role]).is_some_and(|state| {
                    state.freshness == meshway_protocol::state::Freshness::Current
                        && matches!(&state.report,
                            Some(meshway_protocol::state::VisibleReport::Full { report })
                                if report.sockets.values().any(|socket| !socket.endpoints.is_empty()))
                });
                if current {
                    return now_ms();
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        })
        .await??;
        reported.insert(role.name().into(), at.into());
    }
    let mut server_stage = breaks("server").await?;
    server_stage["stoppedAtMs"] = stopped.into();
    server_stage["restartedAtMs"] = restarted.into();
    server_stage["instanceAtMs"] = instances.into();
    server_stage["stateCurrentAtMs"] = reported.into();
    timings.insert("server".into(), server_stage);
    save(&timings).await?;
    for (stage, actor) in [("restart-r", Role::R), ("restart-a", Role::A)] {
        tracing::info!(phase = "reconnect", stage, "restarting one daemon");
        let disrupted = step(&format!("reconnect-{stage}")).await?;
        let restarted = observed(
            &format!("reconnect-restarted-{}", actor.name()),
            STARTUP + IO,
        )
        .await?;
        let direct = observed(&format!("reconnect-direct-{stage}"), RECONNECT * 3).await?;
        timings.insert(
            stage.into(),
            serde_json::json!({
                "disruptedAtMs": disrupted, "restartedAtMs": restarted, "directAtMs": direct
            }),
        );
        save(&timings).await?;
    }
    for (stage, value) in &timings {
        let elapsed = value["directAtMs"]
            .as_u64()
            .zip(value["disruptedAtMs"].as_u64())
            .and_then(|(direct, start)| direct.checked_sub(start))
            .ok_or("reconnect timing out of order")?;
        tracing::info!(phase = "reconnect", stage, elapsed_ms = elapsed, "direct again");
        if Duration::from_millis(elapsed) > RECONNECT {
            return Err(format!("reconnect {stage}: direct again after {elapsed} ms").into());
        }
    }
    signals.write("reconnect-done", &Signal::Ready).await?;
    Ok(())
}
