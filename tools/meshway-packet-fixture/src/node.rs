use crate::{
    Cli, ORIGIN, Result, STARTUP, request, runtime,
    signals::{Role, Signal, Signals},
    tls, wire,
};
use meshway_authority::ExecutionSnapshot;
use meshway_configuration::execution::{BusinessProtocol, BusinessTarget, DemandDecision};
use meshway_control::Control;
use meshway_dataplane::secure::SecureNode;
use meshway_model::state::{Capability, StreamKind, TransportRole};
use meshway_protocol::{DemandRequest, InstanceRequest, enrollment::EnrollmentPhase};
use std::{collections::BTreeMap, time::Duration};

pub async fn run(cli: &Cli, role: Role) -> Result<()> {
    if !cfg!(target_os = "linux") {
        return Err("packet nodes require Linux namespaces".into());
    }
    let signals = Signals::new(&cli.signals);
    let mut control = Control::open(&cli.state, ORIGIN, tls::client(cli).await?)?;
    let result = run_control(&mut control, &signals, role).await;
    let shutdown = tokio::time::timeout(crate::IO, control.shutdown()).await;
    shutdown??;
    let report = result?;
    signals
        .write(
            &format!("report-{}", role.name()),
            Signal::Report {
                role,
                forwarded: report.forwarded,
                delivered: report.delivered,
                tun_removed: report.tun_removed,
            },
        )
        .await?;
    Ok(())
}

async fn sync(control: &mut Control) -> Result<ExecutionSnapshot> {
    control
        .refresh_configuration()
        .await?
        .ok_or("missing node configuration")?;
    Ok(control.execution_snapshot()?)
}

async fn run_control(
    control: &mut Control,
    signals: &Signals,
    role: Role,
) -> Result<runtime::Report> {
    // begin_enrollment persists its intent, making retries the same real request.
    let pending = tokio::time::timeout(STARTUP, async {
        loop {
            match control.begin_enrollment(role.name()).await {
                Ok(value) => return Ok::<_, crate::Error>(value),
                Err(meshway_control::Error::Server(
                    meshway_control_client::ClientError::Network { .. },
                )) => tokio::time::sleep(Duration::from_millis(100)).await,
                Err(error) => return Err(error.into()),
            }
        }
    })
    .await??;
    signals
        .write(
            &format!("enrollment-{}", role.name()),
            Signal::Enrollment {
                role,
                request: pending.id,
            },
        )
        .await?;
    tokio::time::timeout(STARTUP, async {
        loop {
            match control.poll_enrollment().await?.phase {
                EnrollmentPhase::Approved => return Ok::<_, crate::Error>(()),
                EnrollmentPhase::Pending => tokio::time::sleep(Duration::from_millis(100)).await,
                _ => return Err("enrollment left pending/approved states".into()),
            }
        }
    })
    .await??;
    let enrollment = control.redeem_enrollment().await?;
    control.authenticate().await?;
    control
        .acquire_instance(BTreeMap::from([(
            "transport:native_tcp".into(),
            Capability::Transport {
                roles: vec![TransportRole::Dial, TransportRole::Listen],
                streams: vec![StreamKind::Tcp],
            },
        )]))
        .await?;
    signals
        .write(
            &format!("enrolled-{}", role.name()),
            Signal::Enrolled {
                role,
                node: enrollment.node_id,
            },
        )
        .await?;
    tracing::info!(
        role = role.name(),
        phase = "enrolled",
        "node authenticated with its own registered keys"
    );
    if !matches!(signals.wait("go").await?, Signal::Go) {
        return Err("invalid configuration marker".into());
    }
    let initial = sync(control).await?;
    if role == Role::A {
        let cfg = initial.configuration()?;
        let name = meshway_configuration::node_dns_name("b", &cfg.dns.settings.domain)?;
        let discovered = control
            .query_dns(&name)
            .await?
            .record
            .ok_or("B not visible through real discovery")?;
        let lease = control.status().instance.ok_or("no acquired instance")?;
        let demand = control
            .create_demand(&DemandRequest {
                instance: InstanceRequest {
                    instance_id: lease.instance_id,
                    generation: lease.generation,
                },
                request_id: request()?,
                target: BusinessTarget::Mesh {
                    node: discovered.node_id,
                },
                protocol: BusinessProtocol::Udp,
                port: Some(42424.try_into()?),
                profile: "mesh".into(),
                duration_seconds: 120,
            })
            .await?;
        if !matches!(demand.decision, DemandDecision::Permitted { .. }) {
            return Err("UDP demand rejected".into());
        }
    }
    // Relay and terminal discover the demand exclusively from their own compiled
    // execution target. Public rendezvous records carry no path or permission.
    let snapshot = tokio::time::timeout(STARTUP, async {
        loop {
            let snapshot = sync(control).await?;
            if snapshot.configuration()?.permits.len() == 1 {
                return Ok::<_, crate::Error>(snapshot);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await??;
    let configuration = snapshot.configuration()?.clone();
    let permit = configuration
        .permits
        .values()
        .next()
        .ok_or("missing demand")?;
    if permit.paths.len() != 1
        || permit.protocol != BusinessProtocol::Udp
        || permit.port.map(|port| port.get()) != Some(42424)
    {
        return Err("unexpected compiled UDP permit".into());
    }
    let path = permit.paths[0].nodes.clone();
    let index = match role {
        Role::A => 0,
        Role::R => 1,
        Role::B => 2,
    };
    if path.len() != 3 || path[index] != configuration.node_id {
        return Err("compiled path role mismatch".into());
    }
    let all = wire::carriers(&path)?;
    let mut carriers = BTreeMap::new();
    for carrier in all.values() {
        if carrier.dialer == configuration.node_id {
            carriers.insert(carrier.listener, carrier.clone());
        }
        if carrier.listener == configuration.node_id {
            carriers.insert(carrier.dialer, carrier.clone());
        }
    }
    // These checks bind actual provider descriptors to the owned compiled plan;
    // proof verification inside SecureNode authenticates the remote identity.
    for (peer, carrier) in &carriers {
        meshway_policy::admission::admit_carrier(
            &configuration,
            permit.demand_id,
            &path,
            *peer,
            carrier,
        )?;
    }
    let mut secure = SecureNode::new(
        control.dataplane_identity(snapshot)?,
        Default::default(),
        carriers,
    )?;
    let listener = wire::listen(&configuration).await?;
    let mut previous = None;
    let mut next = None;
    let connected = async {
        if index < 2 {
            next = Some(wire::dial(&configuration, all[&path[index]].clone()).await?);
        }
        if index > 0 {
            previous =
                Some(wire::accept(&listener, &configuration, all[&path[index - 1]].clone()).await?);
        }
        Ok::<_, crate::Error>(())
    }
    .await;
    drop(listener);
    let result = match connected {
        Ok(()) => {
            tracing::info!(
                role = role.name(),
                phase = "native-tcp-connected",
                "owned native TCP carrier connected before TUN creation"
            );
            runtime::run(
                role,
                signals,
                &configuration,
                permit.demand_id,
                &path,
                &mut secure,
                &mut previous,
                &mut next,
            )
            .await
        }
        Err(error) => Err(error),
    };
    let previous_close = if let Some(link) = previous.as_mut() {
        link.close().await
    } else {
        Ok(())
    };
    let next_close = if let Some(link) = next.as_mut() {
        link.close().await
    } else {
        Ok(())
    };
    previous_close?;
    next_close?;
    result
}
