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
use std::{future::Future, pin::Pin, sync::Arc};
type SendFuture = Pin<Box<dyn Future<Output = Result<(), Error>> + Send>>;

pub(super) enum Io {
    Session {
        sender: Arc<tokio::sync::Mutex<Sender>>,
        receiver: Receiver,
        incoming: BTreeMap<SocketAddr, Carrier>,
        scopes: BTreeMap<SocketAddr, ChannelScope>,
        report: Box<dyn Fn() -> meshway_transport_udp::Capabilities + Send + Sync>,
    },
    #[cfg(target_os = "linux")]
    Manager {
        manager: Box<Manager>,
        host: Option<Box<meshway_underlay::Host>>,
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
        let buffer = std::env::var("MESHWAY_PERF_UDP_BUFFER")
            .ok()
            .map(|value| value.parse::<u32>())
            .transpose()?
            .unwrap_or(0);
        let endpoint = meshway_transport_udp::UdpTransport::adopt_batch(
            socket,
            &meshway_transport_spec::SocketOptions {
                receive_buffer: buffer,
                send_buffer: buffer,
            },
        )?;
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
            sender: Arc::new(tokio::sync::Mutex::new(sender)),
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
        let mut host = meshway_underlay::Host::open(
            directory,
            meshway_underlay::Options {
                mtu: options.mtu,
                ..Default::default()
            },
        )
        .await?;
        // The production factory only mints data scopes after capture and its
        // execution epoch have been installed. Never substitute a bootstrap scope.
        host.install(identity.snapshot().clone()).await?;
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
            host: Some(Box::new(host)),
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

    #[cfg(target_os = "linux")]
    pub fn take_host(&mut self) -> Option<meshway_underlay::Host> {
        match self {
            Self::Manager { host, .. } => host.take().map(|host| *host),
            Self::Session { .. } => None,
        }
    }

    pub async fn recv(&mut self, data: bool) -> Result<Vec<(Carrier, Frame)>, Error> {
        match self {
            Self::Session {
                receiver, incoming, ..
            } => {
                if !data {
                    std::future::pending().await
                } else {
                    receiver
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
                        .collect()
                }
            }
            #[cfg(target_os = "linux")]
            Self::Manager { manager, .. } => {
                let mut frames = Vec::new();
                for event in manager.recv_batch_with_data(data).await {
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

    pub fn send(&self, frames: Vec<PendingFrame>, fixture: &PublicFixture) -> SendFuture {
        let selected = (|| -> Result<Vec<SendFuture>, Error> {
            let mut sends: Vec<SendFuture> = Vec::new();
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
                            let sender = sender.clone();
                            sends.push(Box::pin(async move {
                                carrier::send(&mut *sender.lock().await, frame, Some(peer)).await?;
                                Ok(())
                            }));
                        }
                    }
                    for (peer, frames) in groups {
                        let sender = sender.clone();
                        let scope = scopes.get(&peer).ok_or("unknown scope")?.clone();
                        sends.push(Box::pin(async move {
                            carrier::send_batch_shared(&sender, frames, peer, scope, None).await?;
                            Ok(())
                        }));
                    }
                }
                #[cfg(target_os = "linux")]
                Self::Manager { manager, .. } => {
                    let mut data = Vec::new();
                    let current = manager.carriers()?;
                    for frame in frames {
                        if current.get(&frame.next()) != Some(frame.carrier()) {
                            // Already-prepared outputs retain their old carrier and
                            // guard. Like the daemon, discard a superseded output;
                            // only future owner admissions use the new selection.
                            meshway_logging::profile::event("l4.output.drop", "selection_changed");
                            continue;
                        }
                        if frame.is_data() {
                            data.push(frame);
                        } else {
                            let send = manager.send_with_receipt(frame);
                            sends.push(Box::pin(async move {
                                send.await?;
                                Ok(())
                            }));
                        }
                    }
                    if !data.is_empty() {
                        let send = manager.send_batch(data);
                        sends.push(Box::pin(async move {
                            send.await?;
                            Ok(())
                        }));
                    }
                }
            }
            Ok(sends)
        })();
        Box::pin(async move {
            for send in selected? {
                send.await?;
            }
            Ok(())
        })
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
            Self::Session { sender, .. } => sender.lock().await.shutdown().await?,
            #[cfg(target_os = "linux")]
            Self::Manager { manager, .. } => {
                manager.shutdown().await?;
            }
        }
        let capabilities = self.capabilities();
        #[cfg(target_os = "linux")]
        if let Self::Manager {
            host: Some(host), ..
        } = self
        {
            host.shutdown(false).await?;
        }
        Ok(capabilities)
    }
}
