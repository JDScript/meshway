//! One bounded packet owner per process, reusing the formal guarded I/O sinks.
use crate::{
    IO, Result, STARTUP,
    application::Applications,
    signals::{Role, Signals},
    wire::{self, Link},
};
use meshway_configuration::execution::NodeConfiguration;
use meshway_dataplane::{
    Packet, PacketError,
    secure::{PendingFrame, Received, SecureNode},
};
use meshway_model::{NodeId, RequestId};
use meshway_tun::{Options, Tun};
use std::time::Duration;
use tokio::sync::watch;

pub struct Report {
    pub forwarded: u64,
    pub delivered: Option<u64>,
    pub tun_removed: Option<bool>,
}

async fn receive(link: &mut Link, secure: &mut SecureNode) -> Result<Received> {
    let frame = tokio::time::timeout(IO, link.receiver.recv()).await??;
    let bytes = wire::inspect(frame, link)?;
    Ok(secure.receive(&link.carrier, bytes)?)
}

async fn handshake(
    role: Role,
    demand: RequestId,
    path: &[NodeId],
    secure: &mut SecureNode,
    previous: &mut Option<Link>,
    next: &mut Option<Link>,
) -> Result<[u8; 32]> {
    let circuit = match role {
        Role::A => {
            let next = next.as_mut().ok_or("missing next carrier")?;
            let initial = secure.start(demand, path.to_vec())?;
            let expected = initial.circuit();
            next.send(initial).await?;
            let Received::Established {
                circuit,
                response: None,
            } = receive(next, secure).await?
            else {
                return Err("invalid source handshake response".into());
            };
            if circuit != expected {
                return Err("handshake circuit mismatch".into());
            }
            circuit
        }
        Role::R => {
            let previous = previous.as_mut().ok_or("missing previous carrier")?;
            let next = next.as_mut().ok_or("missing next carrier")?;
            let Received::Forward(first) = receive(previous, secure).await? else {
                return Err("relay handshake must remain opaque".into());
            };
            let circuit = first.circuit();
            next.send(first).await?;
            let Received::Forward(second) = receive(next, secure).await? else {
                return Err("relay response must remain opaque".into());
            };
            if second.circuit() != circuit {
                return Err("relay circuit mismatch".into());
            }
            previous.send(second).await?;
            circuit
        }
        Role::B => {
            let previous = previous.as_mut().ok_or("missing previous carrier")?;
            let Received::Established {
                circuit,
                response: Some(response),
            } = receive(previous, secure).await?
            else {
                return Err("invalid terminal handshake".into());
            };
            previous.send(response).await?;
            circuit
        }
    };
    tracing::info!(
        role = role.name(),
        phase = "noise-established",
        "registered-key circuit established"
    );
    Ok(circuit)
}

#[allow(clippy::too_many_arguments)]
pub async fn run(
    role: Role,
    signals: &Signals,
    configuration: &NodeConfiguration,
    demand: RequestId,
    path: &[NodeId],
    secure: &mut SecureNode,
    previous: &mut Option<Link>,
    next: &mut Option<Link>,
) -> Result<Report> {
    let circuit = tokio::time::timeout(
        STARTUP,
        handshake(role, demand, path, secure, previous, next),
    )
    .await??;
    let mut report = Report {
        forwarded: 0,
        delivered: (role != Role::R).then_some(0),
        tun_removed: None,
    };
    let mut tun = None;
    let mut tun_identity = None;
    let mut application = None;
    let (stop, stopped) = watch::channel(false);
    if role != Role::R {
        let peer_id = if role == Role::A { path[2] } else { path[0] };
        let peer = configuration
            .peers
            .get(&peer_id)
            .ok_or("missing endpoint peer")?
            .addresses
            .clone();
        let (ipv4, prefix4, ipv6, prefix6) =
            crate::application::prefixes(&configuration.addresses, &peer)?;
        let device = Tun::create(
            Options::new(1280)?
                .with_ipv4(ipv4, prefix4)?
                .with_ipv6(ipv6, prefix6)?,
        )?;
        let index =
            tokio::fs::read_to_string(format!("/sys/class/net/{}/ifindex", device.name())).await?;
        tun_identity = Some((device.name().to_owned(), index));
        let apps = Applications::bind(role, &configuration.addresses).await?;
        let signals = signals.clone();
        application = Some(tokio::spawn(apps.run(role, peer, signals, stopped)));
        tracing::info!(
            role = role.name(),
            phase = "tun-ready",
            interface = device.name(),
            "one dual-stack TUN endpoint installed in this namespace"
        );
        tun = Some(device);
    }
    let outcome = tokio::time::timeout(
        Duration::from_secs(30),
        pump(
            role,
            signals,
            circuit,
            secure,
            previous,
            next,
            tun.as_ref(),
            &mut report,
            &mut application,
        ),
    )
    .await;
    let _ = stop.send(true);
    let application_result = match application {
        Some(mut task) => match tokio::time::timeout(IO, &mut task).await {
            Ok(Ok(result)) => result,
            Ok(Err(error)) => Err(error.into()),
            Err(error) => {
                task.abort();
                let _ = task.await;
                Err(error.into())
            }
        },
        None => Ok(()),
    };
    drop(tun);
    if let Some((name, index)) = tun_identity {
        tokio::time::timeout(IO, async {
            loop {
                match tokio::fs::read_to_string(format!("/sys/class/net/{name}/ifindex")).await {
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        return Ok::<_, crate::Error>(());
                    }
                    Ok(current) if current != index => {
                        return Err("TUN name unexpectedly reused before cleanup proof".into());
                    }
                    Err(error) => return Err(error.into()),
                    Ok(_) => tokio::time::sleep(Duration::from_millis(20)).await,
                }
            }
        })
        .await??;
        report.tun_removed = Some(true);
    }
    application_result?;
    outcome??;
    Ok(report)
}

async fn send(
    frame: PendingFrame,
    previous: &mut Option<Link>,
    next: &mut Option<Link>,
) -> Result<()> {
    for link in [previous, next].into_iter().flatten() {
        if frame.carrier() == &link.carrier && frame.next() == link.remote {
            return link.send(frame).await;
        }
    }
    Err("secure next hop has no actual carrier".into())
}

#[allow(clippy::too_many_arguments)]
async fn pump(
    role: Role,
    signals: &Signals,
    circuit: [u8; 32],
    secure: &mut SecureNode,
    previous: &mut Option<Link>,
    next: &mut Option<Link>,
    tun: Option<&Tun>,
    report: &mut Report,
    application: &mut Option<tokio::task::JoinHandle<Result<()>>>,
) -> Result<()> {
    let mut buffer = [0; 1280];
    let mut tick = tokio::time::interval(Duration::from_millis(50));
    loop {
        enum Event {
            Previous(
                std::result::Result<meshway_transport::Frame, meshway_transport::ReceiveError>,
            ),
            Next(std::result::Result<meshway_transport::Frame, meshway_transport::ReceiveError>),
            Tun(std::result::Result<usize, meshway_tun::Error>),
            Tick,
        }
        let event = tokio::select! {
            value = async { previous.as_mut().expect("guarded previous").receiver.recv().await }, if previous.is_some() => Event::Previous(value),
            value = async { next.as_mut().expect("guarded next").receiver.recv().await }, if next.is_some() => Event::Next(value),
            value = async { tun.expect("guarded TUN").recv(&mut buffer).await }, if tun.is_some() => Event::Tun(value),
            _ = tick.tick() => Event::Tick,
        };
        let received = match event {
            Event::Tick => {
                if application.as_ref().is_some_and(|task| task.is_finished()) {
                    let task = application.take().ok_or("application task disappeared")?;
                    task.await??;
                    if role == Role::B {
                        return Err("echo application ended before stop".into());
                    }
                }
                if signals.done().await? {
                    return Ok(());
                }
                continue;
            }
            Event::Tun(result) => {
                let length = result?;
                let packet = match Packet::parse(buffer[..length].to_vec()) {
                    Ok(packet) => packet,
                    // The kernel may emit IPv6 neighbor/router solicitation;
                    // the production parser does not admit it as business IP.
                    Err(PacketError::Unsupported) => continue,
                    Err(error) => return Err(error.into()),
                };
                let frame = secure.send(circuit, packet)?;
                send(frame, previous, next).await?;
                report.forwarded += 1;
                continue;
            }
            Event::Previous(result) => {
                let frame = match result {
                    Ok(frame) => frame,
                    Err(error) => {
                        if signals.done().await? {
                            return Ok(());
                        }
                        return Err(error.into());
                    }
                };
                let link = previous.as_ref().ok_or("previous carrier disappeared")?;
                secure.receive(&link.carrier, wire::inspect(frame, link)?)?
            }
            Event::Next(result) => {
                let frame = match result {
                    Ok(frame) => frame,
                    Err(error) => {
                        if signals.done().await? {
                            return Ok(());
                        }
                        return Err(error.into());
                    }
                };
                let link = next.as_ref().ok_or("next carrier disappeared")?;
                secure.receive(&link.carrier, wire::inspect(frame, link)?)?
            }
        };
        match received {
            Received::Forward(frame) => {
                if role != Role::R {
                    return Err("endpoint unexpectedly relayed".into());
                }
                send(frame, previous, next).await?;
                report.forwarded += 1;
            }
            Received::Packet(delivery) => {
                if role == Role::R {
                    return Err("relay received plaintext application data".into());
                }
                tokio::time::timeout(
                    IO,
                    meshway_dataplane::carrier::deliver(
                        tun.ok_or("missing endpoint TUN")?,
                        delivery,
                    ),
                )
                .await??;
                *report
                    .delivered
                    .as_mut()
                    .ok_or("missing endpoint counter")? += 1;
            }
            Received::Established { .. } => return Err("unexpected second handshake".into()),
        }
    }
}
