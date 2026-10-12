//! Controller side of the default row after the shared startup and traffic:
//! ICMP-only and full Access revocation, restore, interface change, Node HTTP
//! outage to instance expiry, and automatic recovery.
use super::*;

pub(super) async fn run(context: scenario::Context<'_>) -> Result<()> {
    let scenario::Context {
        cli,
        signals,
        client,
        network,
        token,
        nodes,
        availability,
        ..
    } = context;
    let original = source(client, network, token).await?;
    let mut allowed = original.configuration.clone();
    // Isolate ICMP revocation from the later relay address change: changing a
    // Node's addresses intentionally invalidates its execution continuity.
    let mut business_only = allowed.clone();
    // The relay no longer owns a Mesh IPv6 address, but must still forward
    // authorized IPv6 ciphertext and retain negative IPv6 capture locally.
    allowed
        .nodes
        .get_mut(&nodes[&Role::R])
        .ok_or("relay configuration absent")?
        .addresses
        .ipv6 = Input::Value(None);
    business_only
        .access
        .retain(|rule| !rule.ip.iter().any(|ip| ip == "icmp:*"));
    if business_only.access.len() != 1
        || business_only.access[0].ip != ["tcp:42425", "udp:42424", "udp:42428"]
    {
        return Err("ICMP-only revocation widened or changed business permissions".into());
    }
    let revision = apply(client, network, token, original.revision, &business_only).await?;
    signals
        .write("ping-icmp-revoked", &Signal::Revision { revision })
        .await?;
    applied(
        cli,
        client,
        network,
        token,
        nodes,
        revision,
        ("icmp-revoked", None),
    )
    .await?;
    staged(signals, "icmp-revoked").await?;
    let Signal::PingRun { run } = signals
        .wait_for("ping-udp-tcp-only", crate::ping::STAGE_LIMIT + IO)
        .await?
    else {
        return Err("missing UDP/TCP-only diagnostic denial".into());
    };
    run.validate(&[nodes[&Role::A], nodes[&Role::R], nodes[&Role::B]])?;
    crate::ping_management::denied(cli, client, network, token, nodes, &run).await?;
    let mut denied = allowed.clone();
    denied.access.clear();
    let revision = apply(client, network, token, revision, &denied).await?;
    signals
        .write("revoked", &Signal::Revision { revision })
        .await?;
    applied(
        cli,
        client,
        network,
        token,
        nodes,
        revision,
        ("revoked", None),
    )
    .await?;
    staged(signals, "revoked").await?;
    if !matches!(
        signals
            .wait_for("revoked-traffic", STARTUP + crate::ping::STAGE_LIMIT)
            .await?,
        Signal::Ready
    ) {
        return Err("invalid revoked traffic/diagnostic completion".into());
    }
    let Signal::PingRun { run } = signals.wait("ping-revoked").await? else {
        return Err("missing permission-denied CLI evidence".into());
    };
    run.validate(&[nodes[&Role::A], nodes[&Role::R], nodes[&Role::B]])?;
    tracing::info!(
        phase = "access-revoked",
        "Server NodeState applied and OS echo rejection verified"
    );
    let revision = apply(client, network, token, revision, &allowed).await?;
    signals
        .write("restored", &Signal::Revision { revision })
        .await?;
    applied(
        cli,
        client,
        network,
        token,
        nodes,
        revision,
        ("restored", None),
    )
    .await?;
    interface_change::run(cli, signals, client, network, token, nodes).await?;
    for role in Role::ALL {
        if !matches!(
            signals
                .wait_for(
                    &format!("lease-ready-{}", role.name()),
                    STARTUP + crate::ping::STAGE_LIMIT * 2
                )
                .await?,
            Signal::Ready
        ) {
            return Err("invalid restored traffic/CLI lifecycle completion".into());
        }
    }
    if cli.traversal_fallback {
        measured_links(cli, client, network, token, nodes, true, "restored-carrier").await?;
    }
    signals.ready("ping-outage-ready").await?;
    let closed_at_ms = availability.close().await?;
    signals
        .write(
            "http-outage",
            &Signal::HttpAvailability {
                available: false,
                observed_at_ms: closed_at_ms,
            },
        )
        .await?;
    tracing::info!(
        phase = "http-outage",
        closed_at_ms,
        "Node HTTP admission closed; real Server owner remains running"
    );
    let mut expired = BTreeMap::new();
    for role in Role::ALL {
        let Signal::Expired {
            role: claimed,
            evidence,
        } = signals
            .wait_for(
                &format!("instance-expired-{}", role.name()),
                cli.netmap_validity.as_std() + Duration::from_secs(75),
            )
            .await?
        else {
            return Err("missing instance-expiry evidence".into());
        };
        evidence.validate()?;
        if claimed != role
            || evidence.closed_at_ms != closed_at_ms
            || evidence.before.identity.as_ref().is_none_or(|identity| {
                identity.node_id != nodes[&role] || identity.network_id != network
            })
        {
            return Err("instance-expiry evidence identity mismatch".into());
        }
        let view = client
            .get_node_state(
                &GetNodeStatePath {
                    network_id: network,
                    node_id: nodes[&role],
                },
                token,
            )
            .await?;
        if view
            .nodes
            .get(&nodes[&role])
            .is_none_or(|state| state.freshness == Freshness::Current)
        {
            return Err(
                "Server still reports current Node State after gated instance expiry".into(),
            );
        }
        tokio::fs::write(
            cli.evidence
                .join(format!("state-expired-{}.json", role.name())),
            serde_json::to_vec_pretty(&view)?,
        )
        .await?;
        expired.insert(role, evidence);
    }
    let (reopened_at_ms, rejected_requests) = availability.open()?;
    tokio::fs::write(
        cli.evidence.join("http-outage.json"),
        serde_json::to_vec_pretty(&crate::recovery::Window {
            closed_at_ms,
            reopened_at_ms,
            rejected_node_requests: rejected_requests,
            response_status: 503,
        })?,
    )
    .await?;
    signals
        .write(
            "recovery-open",
            &Signal::HttpAvailability {
                available: true,
                observed_at_ms: reopened_at_ms,
            },
        )
        .await?;
    let mut recovered = BTreeMap::new();
    for role in Role::ALL {
        let Signal::Status {
            role: claimed,
            status,
        } = signals
            .wait_for(
                &format!("recovery-applied-{}", role.name()),
                Duration::from_secs(75),
            )
            .await?
        else {
            return Err("missing automatically recovered status".into());
        };
        if claimed != role {
            return Err("recovered role mismatch".into());
        }
        expired[&role].validate_recovered(&status, reopened_at_ms)?;
        recovered.insert(role, *status);
    }
    applied(
        cli,
        client,
        network,
        token,
        nodes,
        revision,
        ("recovered", Some(&recovered)),
    )
    .await?;
    signals
        .write("recovery-state-verified", &Signal::Ready)
        .await?;
    for role in Role::ALL {
        if !matches!(
            signals
                .wait_for(
                    &format!("recovery-done-{}", role.name()),
                    Duration::from_secs(150) + crate::ping::STAGE_LIMIT
                )
                .await?,
            Signal::Ready
        ) {
            return Err("invalid recovery traffic completion".into());
        }
    }
    measured_links(
        cli,
        client,
        network,
        token,
        nodes,
        cli.traversal_fallback,
        "recovered",
    )
    .await?;
    tracing::info!(
        phase = "automatic-recovery-verified",
        "New instances, real applied State and fresh OS flows verified; stopping actual Server next"
    );
    Ok(())
}
