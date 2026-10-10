//! Controller-only authorization and shared-user-subscription assertions. The
//! actor receives only typed public evidence, never the management credential.
use crate::{
    Cli, Error, Result,
    ping::{Run, Stage},
    signals::{Role, Signal, Signals},
};
use meshway_configuration::execution::ObservationServiceScope;
use meshway_control_client::{
    CancelObservationPath, Client, CreateObservationPath, GetObservationPath, RenewObservationPath,
};
use meshway_model::{
    InstanceId, NetworkId, NodeId,
    observations::{EndpointKind, LinkAcquisition, MetricKind},
};
use meshway_protocol::{
    CreateObservation,
    observations::{ObservationPath, ObservationTask},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SharedEvidence {
    pub created: ObservationTask,
    pub cli: Run,
    pub observed_after_cli: ObservationTask,
    pub cancellation_verified: bool,
}

/// Public pre/post status can be unavailable during a normal epoch handoff.
/// Keep that absence in the evidence and require a real observed anchor, then
/// correlate it with the immutable offer or the actual Server task instance.
fn observed_instance(run: &Run) -> Result<InstanceId> {
    match (run.before.instance, run.after.instance) {
        (Some(before), Some(after)) if before != after => {
            Err("ping crossed source instances".into())
        }
        (Some(instance), _) | (_, Some(instance)) => Ok(instance),
        (None, None) => Err("ping source instance absent from both lifecycle observations".into()),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct NatChannelProof {
    stage: Stage,
    measurement_id: meshway_model::EventId,
    source: NodeId,
    channel: meshway_model::observations::ProbeChannel,
    authenticated_offer: meshway_protocol::connectivity::UdpTraversalOffer,
}

fn request(nodes: &BTreeMap<Role, NodeId>) -> Result<CreateObservation> {
    Ok(CreateObservation {
        subscription_id: crate::request()?.to_string().parse()?,
        source_node_id: nodes[&Role::A],
        target_node_id: nodes[&Role::B],
        selection: ObservationPath::Auto,
        service: ObservationServiceScope::IcmpEcho,
        metrics: vec![MetricKind::Rtt, MetricKind::ProbeLoss],
        interval_ms: 1000,
    })
}

/// Real user API must reject the same ICMP request while UDP/TCP still work.
pub async fn denied(
    cli: &Cli,
    client: &Client,
    network: NetworkId,
    token: &str,
    nodes: &BTreeMap<Role, NodeId>,
    run: &Run,
) -> Result<()> {
    let request = request(nodes)?;
    let response = client
        .create_observation(
            &CreateObservationPath {
                network_id: network,
            },
            &request,
            token,
        )
        .await;
    match response {
        Err(meshway_control_client::ClientError::Http { status: 403, .. }) => {}
        Ok(_) => {
            client
                .cancel_observation(
                    &CancelObservationPath {
                        network_id: network,
                        subscription_id: request.subscription_id,
                    },
                    token,
                )
                .await?;
            return Err("user API granted ICMP under UDP/TCP-only Access".into());
        }
        _ => return Err("user API did not return typed HTTP permission rejection".into()),
    }
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct PermissionEvidence<'a> {
        request: CreateObservation,
        user_http_status: u16,
        cli: &'a Run,
    }
    tokio::fs::write(
        cli.evidence.join("ping-permission.json"),
        serde_json::to_vec_pretty(&PermissionEvidence {
            request,
            user_http_status: 403,
            cli: run,
        })?,
    )
    .await?;
    Ok(())
}

pub async fn initial(
    cli: &Cli,
    signals: &Signals,
    client: &Client,
    network: NetworkId,
    token: &str,
    nodes: &BTreeMap<Role, NodeId>,
    audit: &crate::traversal::Audit,
) -> Result<()> {
    signals.write("ping-go", &Signal::Ready).await?;
    let path = vec![nodes[&Role::A], nodes[&Role::R], nodes[&Role::B]];
    let mut nat = Vec::new();
    for (name, stage) in [
        ("hostname", Stage::Hostname),
        ("ipv4", Stage::Ipv4),
        ("ipv6", Stage::Ipv6),
        ("direct", Stage::Direct),
    ] {
        let Signal::PingRun { run } = signals
            .wait_for(&format!("ping-{name}"), crate::ping::STAGE_LIMIT)
            .await?
        else {
            return Err("missing typed CLI ping stage".into());
        };
        if run.stage != stage {
            return Err("CLI ping stage mismatch".into());
        }
        run.validate(if stage == Stage::Direct {
            &path[..2]
        } else {
            &path
        })?;
        let source_instance = observed_instance(&run)?;
        for sample in run.samples() {
            let channels = sample
                .probe
                .as_ref()
                .and_then(|probe| probe.channels.as_ref())
                .ok_or("CLI ping missing physical channel")?;
            for channel in [
                &channels.request,
                channels
                    .reply
                    .as_ref()
                    .ok_or("CLI ping missing reply channel")?,
            ] {
                if channel.transport != cli.transport.protocol() {
                    return Err("CLI ping used wrong carrier".into());
                }
                match (&channel.acquisition, cli.traversal) {
                    (
                        LinkAcquisition::CoordinatedUdp {
                            endpoint_source: EndpointKind::Stun,
                            ..
                        },
                        crate::TraversalMode::Nat,
                    ) => {
                        nat.push(NatChannelProof {
                            stage,
                            measurement_id: sample.measurement_id,
                            source: nodes[&Role::A],
                            channel: channel.clone(),
                            authenticated_offer: audit.verify_ping_channel(
                                channel,
                                nodes[&Role::A],
                                source_instance,
                            )?,
                        });
                    }
                    (
                        LinkAcquisition::PublishedEndpoint
                        | LinkAcquisition::LocalEndpoint
                        | LinkAcquisition::AcceptedInbound,
                        crate::TraversalMode::Off,
                    ) => {
                        let underlay = crate::signals::underlay(cli).await?;
                        let remote = channel.remote_address.get();
                        if cli.reported_endpoints
                            && matches!(channel.acquisition, LinkAcquisition::PublishedEndpoint)
                        {
                            return Err(
                                "reported-only fixture acquired a configured endpoint".into()
                            );
                        }
                        if remote.ip() != std::net::IpAddr::V4(underlay.actors[&Role::R])
                            || (matches!(
                                channel.acquisition,
                                LinkAcquisition::PublishedEndpoint | LinkAcquisition::LocalEndpoint
                            ) && remote.port() != 35000)
                        {
                            return Err("CLI static channel remote endpoint mismatch".into());
                        }
                    }
                    _ => {
                        return Err(
                            "CLI ping acquisition provenance does not match actual fixture mode"
                                .into(),
                        );
                    }
                }
            }
        }
    }
    if cli.traversal == crate::TraversalMode::Nat {
        tokio::fs::write(
            cli.evidence.join("ping-nat-channel-proofs.json"),
            serde_json::to_vec_pretty(&nat)?,
        )
        .await?;
    }
    let created = client
        .create_observation(
            &CreateObservationPath {
                network_id: network,
            },
            &request(nodes)?,
            token,
        )
        .await?;
    let subscription = created
        .subscription_id
        .ok_or("user observation subscription missing")?;
    let cancel_path = CancelObservationPath {
        network_id: network,
        subscription_id: subscription,
    };
    let result = async {
        if created.source_node_id != nodes[&Role::A]
            || created.target_node_id != nodes[&Role::B]
            || created.service != ObservationServiceScope::IcmpEcho
            || created.selection != ObservationPath::Auto
            || created.metrics != vec![MetricKind::Rtt, MetricKind::ProbeLoss]
            || created.interval_ms != 1000
        {
            return Err("Console request does not match real daemon defaults".into());
        }
        signals.write("ping-shared-go", &Signal::Ready).await?;
        let deadline = Instant::now() + crate::remaining(crate::ping::STAGE_LIMIT)?;
        let mut renew = Instant::now() + Duration::from_secs(10);
        let run = loop {
            if let Some(signal) = signals.read("ping-shared").await? {
                let Signal::PingRun { run } = signal else {
                    return Err("invalid shared CLI ping signal".into());
                };
                break run;
            }
            if Instant::now() >= deadline {
                return Err("shared CLI completion deadline exceeded".into());
            }
            if Instant::now() >= renew {
                client
                    .renew_observation(
                        &RenewObservationPath {
                            network_id: network,
                            subscription_id: subscription,
                        },
                        token,
                    )
                    .await?;
                renew = Instant::now() + Duration::from_secs(10);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        };
        run.validate(&path)?;
        let (local_subscription, task) = run.accepted()?;
        if run.stage != Stage::Shared
            || task != created.id
            || local_subscription == subscription
            || observed_instance(&run)? != created.instance_id
        {
            return Err(
                "Console and CLI did not reuse one task with distinct subscriptions".into(),
            );
        }
        let get = GetObservationPath {
            network_id: network,
            subscription_id: subscription,
        };
        let mut get_evidence_count = 0u16;
        tracing::info!(
            phase = "ping-shared-post-cli",
            observation = %created.id,
            cli_finished_at_ms = run.finished_at_ms,
            "Awaiting a new user-visible measurement after CLI completion"
        );
        let observed = tokio::time::timeout(crate::remaining(Duration::from_secs(15))?, async {
            loop {
                if Instant::now() >= renew {
                    client
                        .renew_observation(
                            &RenewObservationPath {
                                network_id: network,
                                subscription_id: subscription,
                            },
                            token,
                        )
                        .await?;
                    renew = Instant::now() + Duration::from_secs(10);
                }
                let get_started_at_ms = crate::recovery::now_ms()?;
                let get_started = Instant::now();
                let list = client.get_observation(&get, token).await?;
                if list.items.len() != 1 {
                    return Err("shared source task owner was duplicated or cancelled".into());
                }
                let task = list.items.into_iter().next().ok_or("shared task missing")?;
                // Only bounded public lifecycle/sample metadata. In particular,
                // never emit the management credential or configuration payload.
                if get_evidence_count < 160 {
                    get_evidence_count += 1;
                    tracing::info!(
                        phase = "ping-shared-user-get",
                        poll = get_evidence_count,
                        get_started_at_ms,
                        get_elapsed_ms = get_started.elapsed().as_millis() as u64,
                        cli_finished_at_ms = run.finished_at_ms,
                        observation = %task.id,
                        observation_phase = ?task.phase,
                        netmap_serial = task.netmap.serial.get(),
                        segment = ?task.path_segment,
                        measurement = ?task.latest.as_ref().map(|sample| sample.measurement_id),
                        window_start_ms = ?task.latest.as_ref().map(|sample| sample.window_start.timestamp_millis()),
                        window_end_ms = ?task.latest.as_ref().map(|sample| sample.window_end.timestamp_millis()),
                        probe_started_ms = ?task.latest.as_ref().and_then(|sample| sample.probe.as_ref()).map(|probe| probe.started_at.timestamp_millis()),
                        cli_measurement = task.latest.as_ref().is_some_and(|sample| run.samples().any(|old| old.measurement_id == sample.measurement_id)),
                        "Post-CLI user observation GET evidence"
                    );
                }
                if task.id != created.id
                    || task.instance_id != created.instance_id
                    || task.subscription_id != Some(subscription)
                {
                    return Err("CLI exit changed user subscription/source owner".into());
                }
                if let Some(sample) = &task.latest
                    && sample.window_start.timestamp_millis() > i64::try_from(run.finished_at_ms)?
                    && !run
                        .samples()
                        .any(|old| old.measurement_id == sample.measurement_id)
                {
                    crate::ping::validate_sample(sample, &path)?;
                    return Ok::<_, Error>(task);
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await??;
        Ok::<_, Error>((*run, observed))
    }
    .await;
    // The same user always cancels its own subscription, including failed checks.
    let cancelled = client.cancel_observation(&cancel_path, token).await;
    let (run, observed) = result?;
    cancelled?;
    let after = client
        .get_observation(
            &GetObservationPath {
                network_id: network,
                subscription_id: subscription,
            },
            token,
        )
        .await;
    match after {
        Ok(view)
            if view.items.len() == 1
                && view.items[0].id == created.id
                && view.items[0].subscription_id == Some(subscription)
                && view.items[0].phase
                    == meshway_protocol::observations::ObservationPhase::Cancelled
                && view.items[0].latest.is_none() => {}
        Err(meshway_control_client::ClientError::Http { status: 404, .. }) => {}
        _ => return Err("cancelled user observation retained an active result".into()),
    }
    tokio::fs::write(
        cli.evidence.join("ping-shared-user.json"),
        serde_json::to_vec_pretty(&SharedEvidence {
            created,
            cli: run,
            observed_after_cli: observed,
            cancellation_verified: true,
        })?,
    )
    .await?;
    signals
        .write("ping-shared-verified", &Signal::Ready)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_projection_needs_a_real_anchor_and_never_changes_retained_evidence() {
        let mut run = crate::ping::roundtrip_run(1);
        let instance = "00000000-0000-4000-8000-000000000001".parse().unwrap();
        run.after.instance = Some(instance);
        run.before.instance = None;
        assert_eq!(observed_instance(&run).unwrap(), instance);
        assert!(run.before.instance.is_none());
        run.before.instance = Some(instance);
        run.after.instance = None;
        assert_eq!(observed_instance(&run).unwrap(), instance);
        assert!(run.after.instance.is_none());
        run.before.instance = None;
        assert!(observed_instance(&run).is_err());
    }

    #[test]
    fn different_known_instances_are_not_a_publication_handoff() {
        let mut run = crate::ping::roundtrip_run(1);
        run.before.instance = Some("00000000-0000-4000-8000-000000000001".parse().unwrap());
        run.after.instance = Some("00000000-0000-4000-8000-00000000ffff".parse().unwrap());
        assert_ne!(run.before.instance, run.after.instance);
        assert!(observed_instance(&run).is_err());
    }
}
