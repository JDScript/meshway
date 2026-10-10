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
use meshway_configuration::NetworkConfigurationInput;
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
    let allowed = NetworkConfigurationInput::from(&original.configuration);
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
