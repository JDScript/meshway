use crate::{Error, NodeOptions, Stage};
use meshway_authority::DataPlaneIdentity;
#[cfg(target_os = "linux")]
use meshway_connectivity::{Event, Manager};
use meshway_dataplane::{
    carrier,
    secure::{Carrier, PendingFrame},
};
use meshway_model::NodeId;
use meshway_static_authority::public_fixture::PublicFixture;
use meshway_transport::{ChannelScope, Frame, Limits, Receiver, Sender};
use std::{collections::BTreeMap, net::SocketAddr, path::Path};

pub(super) enum Io {
    Session {
        sender: Sender,
        receiver: Receiver,
        incoming: BTreeMap<SocketAddr, Carrier>,
        scopes: BTreeMap<SocketAddr, ChannelScope>,
        report: Box<dyn Fn() -> meshway_transport_udp::Capabilities + Send + Sync>,
    },
    #[cfg(target_os = "linux")]
    Manager {
        manager: Box<Manager>,
        host: meshway_underlay::Host,
    },
}

impl Io {
    pub async fn start(
        options: &NodeOptions,
        fixture: &PublicFixture,
        local: NodeId,
        identity: DataPlaneIdentity,
        material: &Path,
    ) -> Result<Self, Error> {
        if matches!(options.stage, Stage::Manager) {
            return Self::manager(options, fixture, local, identity, material).await;
        }
        let socket = tokio::net::UdpSocket::bind(options.bind).await?;
        let endpoint =
            meshway_transport_udp::UdpTransport::adopt_batch(socket, &Default::default())?;
        let report = Box::new(endpoint.capability_report());
        let frame_limit = (options.mtu + 2048).min(65_507);
        let (sender, receiver) = endpoint
            .into_session(Limits::new(frame_limit, 1024, frame_limit * 1024, 1, 1024)?)
            .into_split();
        let incoming = fixture
            .carriers_for(local)?
            .into_iter()
            .map(|(id, carrier)| {
                Ok((
                    *fixture.sockets.get(&id).ok_or("missing neighbour socket")?,
                    carrier,
                ))
            })
            .collect::<Result<BTreeMap<_, _>, Error>>()?;
        let scopes = incoming
            .keys()
            .map(|peer| (*peer, ChannelScope::new()))
            .collect();
        Ok(Self::Session {
            sender,
            receiver,
            incoming,
            scopes,
            report,
        })
    }

    #[cfg(target_os = "linux")]
    async fn manager(
        options: &NodeOptions,
        fixture: &PublicFixture,
        local: NodeId,
        identity: DataPlaneIdentity,
        material: &Path,
    ) -> Result<Self, Error> {
        let directory = material
            .parent()
            .ok_or("material directory missing")?
            .join("underlay");
        let directory = std::sync::Arc::new(meshway_private_storage::Directory::open(
            &directory,
            "owner.lock",
        )?);
        let host = meshway_underlay::Host::open(
            directory,
            meshway_underlay::Options {
                mtu: options.mtu,
                ..Default::default()
            },
        )
        .await?;
        let manager = Manager::start(
            identity,
            std::sync::Arc::new(host.sockets()),
            Default::default(),
        )
        .await?;
        let expected = fixture.carriers_for(local)?.len();
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                if manager.carriers()?.len() == expected {
                    return Ok::<_, Error>(());
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await??;
        Ok(Self::Manager {
            manager: Box::new(manager),
            host,
        })
    }

    #[cfg(not(target_os = "linux"))]
    async fn manager(
        _: &NodeOptions,
        _: &PublicFixture,
        _: NodeId,
        _: DataPlaneIdentity,
        _: &Path,
    ) -> Result<Self, Error> {
        Err("Manager measurement requires isolated Linux underlay".into())
    }

    pub fn carriers(
        &self,
        fixture: &PublicFixture,
        local: NodeId,
    ) -> Result<BTreeMap<NodeId, Carrier>, Error> {
        match self {
            Self::Session { .. } => fixture.carriers_for(local),
            #[cfg(target_os = "linux")]
            Self::Manager { manager, .. } => Ok(manager.carriers()?),
        }
    }

    pub async fn recv(&mut self) -> Result<Vec<(Carrier, Frame)>, Error> {
        match self {
            Self::Session {
                receiver, incoming, ..
            } => receiver
                .recv_batch()
                .await?
                .into_iter()
                .map(|frame| {
                    let carrier = incoming
                        .get(&frame.peer.ok_or("missing source")?)
                        .ok_or("unexpected secure UDP source")?
                        .clone();
                    Ok((carrier, frame))
                })
                .collect(),
            #[cfg(target_os = "linux")]
            Self::Manager { manager, .. } => {
                let mut frames = Vec::new();
                for event in manager.recv_batch().await {
                    match event {
                        Event::Frame { link, frame, .. } => frames.push((link.carrier, frame)),
                        Event::Changed => {}
                        Event::Closed => return Err("Manager closed during measurement".into()),
                    }
                }
                Ok(frames)
            }
        }
    }

    pub async fn send(
        &mut self,
        frames: Vec<PendingFrame>,
        fixture: &PublicFixture,
    ) -> Result<(), Error> {
        match self {
            Self::Session { sender, scopes, .. } => {
                let mut groups = BTreeMap::<SocketAddr, Vec<PendingFrame>>::new();
                for frame in frames {
                    let peer = *fixture
                        .sockets
                        .get(&frame.next())
                        .ok_or("unknown next hop")?;
                    if frame.is_data() {
                        groups.entry(peer).or_default().push(frame);
                    } else {
                        carrier::send(sender, frame, Some(peer)).await?;
                    }
                }
                for (peer, frames) in groups {
                    carrier::send_batch(
                        sender,
                        frames,
                        Some(peer),
                        scopes.get(&peer).ok_or("unknown scope")?.clone(),
                        None,
                    )
                    .await?;
                }
            }
            #[cfg(target_os = "linux")]
            Self::Manager { manager, .. } => {
                let mut data = Vec::new();
                for frame in frames {
                    if frame.is_data() {
                        data.push(frame);
                    } else {
                        manager.send(frame).await?;
                    }
                }
                if !data.is_empty() {
                    manager.send_batch(data).await?;
                }
            }
        }
        Ok(())
    }

    pub fn capabilities(&self) -> serde_json::Value {
        match self {
            Self::Session { report, .. } => {
                let udp = report();
                serde_json::json!({"udp": {"segment": udp.udp_segment, "gro": udp.udp_gro,
                    "sendmmsg": udp.sendmmsg, "recvmmsg": udp.recvmmsg, "gsoSends": udp.gso_sends,
                    "groReceives": udp.gro_receives, "mmsgSends": udp.mmsg_sends, "mmsgReceives": udp.mmsg_receives}})
            }
            #[cfg(target_os = "linux")]
            Self::Manager { .. } => serde_json::json!({"udp": null}),
        }
    }

    pub async fn shutdown(mut self) -> Result<serde_json::Value, Error> {
        match &mut self {
            Self::Session { sender, .. } => sender.shutdown().await?,
            #[cfg(target_os = "linux")]
            Self::Manager { manager, .. } => {
                manager.shutdown().await?;
            }
        }
        let capabilities = self.capabilities();
        #[cfg(target_os = "linux")]
        if let Self::Manager { host, .. } = self {
            host.shutdown(false).await?;
        }
        Ok(capabilities)
    }
}
