use crate::{
    Cli, ORIGIN, Result, STARTUP, runtime,
    signals::{Role, Signal, Signals},
    tls, wire,
};
use meshway_control::Control;
use meshway_dataplane::secure::SecureNode;
use meshway_model::state::{Capability, StreamKind, TransportRole};
use meshway_protocol::enrollment::EnrollmentPhase;
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

/// One sync; Control's verified netmap as desired content.
async fn sync(control: &mut Control) -> Result<meshway_engine::Desired> {
    let netmap = control.sync(&mut Facts).await?.netmap;
    Ok(
        meshway_engine::Desired::new(&netmap.content, &Default::default())
            .map_err(|_| "invalid netmap")?,
    )
}

struct Facts;
impl meshway_control::PeerFactsSink for Facts {
    fn publish(&mut self, _: meshway_control::KnownPeers) {}
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
    let desired = sync(control).await?;
    let configuration = desired.netmap().clone();
    let named = |name| {
        configuration
            .projection
            .configuration
            .nodes
            .iter()
            .find(|(_, settings)| settings.hostname == name)
            .map(|(id, _)| *id)
            .ok_or("fixture Node missing from netmap")
    };
    let (a, b) = (named("a")?, named("b")?);
    let target = meshway_dataplane::Target::Mesh { node: b };
    let compiler =
        meshway_policy::compile::netmap::LocalCompiler::new(&configuration, &Default::default());
    let paths = compiler.transit(a, &target)?;
    if paths.len() != 1 {
        return Err("expected one complete netmap path".into());
    }
    let path = paths[0].nodes.clone();
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
        meshway_policy::admission::netmap::admit_adjacency(
            &compiler,
            &configuration,
            *peer,
            carrier,
        )?;
    }
    let mut secure = SecureNode::new(
        &desired,
        control.dataplane_keys()?,
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
