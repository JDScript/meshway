//! Native TCP socket ownership; all guarded transmission uses the production sink.
use crate::{IO, Result, STARTUP};
use meshway_configuration::netmap::NodeNetmap;
use meshway_dataplane::secure::{Carrier, PendingFrame};
use meshway_model::NodeId;
use meshway_model::TransportProtocol;
use meshway_transport::{Receiver, Sender};
use meshway_transport_spec::{DialParameters, Listener, SocketOptions, TcpOptions};
use meshway_transport_tcp::TcpTransport;
use std::{collections::BTreeMap, net::SocketAddr};
use tokio::net::{TcpListener, TcpStream};

pub struct Link {
    pub sender: Sender,
    pub receiver: Receiver,
    pub carrier: Carrier,
    pub peer: SocketAddr,
    pub remote: NodeId,
}
impl Link {
    fn adopt(
        stream: TcpStream,
        carrier: Carrier,
        remote: NodeId,
        socket: &SocketOptions,
        tcp: &TcpOptions,
    ) -> Result<Self> {
        let transport = TcpTransport::adopt(stream, socket, tcp)?;
        let peer = transport.peer_addr()?;
        let limits = meshway_transport::Limits::new(128 * 1024, 8, 1024 * 1024, 8, 1)?;
        let (sender, receiver) = transport.into_session(limits)?.into_split();
        Ok(Self {
            sender,
            receiver,
            carrier,
            peer,
            remote,
        })
    }
    pub async fn send(&mut self, frame: PendingFrame) -> Result<()> {
        if frame.carrier() != &self.carrier || frame.next() != self.remote {
            return Err("outgoing carrier mismatch".into());
        }
        tokio::time::timeout(
            IO,
            meshway_dataplane::carrier::send(&mut self.sender, frame, Some(self.peer)),
        )
        .await??;
        Ok(())
    }
    pub async fn close(&mut self) -> Result<()> {
        tokio::time::timeout(IO, self.sender.shutdown()).await??;
        Ok(())
    }
}

pub fn carriers(path: &[NodeId]) -> Result<BTreeMap<NodeId, Carrier>> {
    if path.len() != 3 {
        return Err("expected one three-node path".into());
    }
    Ok(path
        .windows(2)
        .map(|hop| {
            (
                hop[0],
                Carrier {
                    dialer: hop[0],
                    listener: hop[1],
                    listener_key: "tcp".into(),
                    protocol: TransportProtocol::NativeTcp,
                },
            )
        })
        .collect())
}

pub async fn listen(configuration: &NodeNetmap) -> Result<TcpListener> {
    let prepared = configuration
        .self_
        .listeners
        .get("tcp")
        .ok_or("missing TCP listener")?;
    let Listener::NativeTcp(options) = prepared.declaration() else {
        return Err("wrong listener provider".into());
    };
    if !options.enabled || options.listen_port.get() != 35000 || !options.bind_interface.is_empty()
    {
        return Err("unexpected listener options".into());
    }
    Ok(TcpListener::bind(SocketAddr::new(options.listen, options.listen_port.get())).await?)
}

pub async fn accept(
    listener: &TcpListener,
    configuration: &NodeNetmap,
    carrier: Carrier,
) -> Result<Link> {
    let prepared = configuration
        .self_
        .listeners
        .get(&carrier.listener_key)
        .ok_or("missing actual listener")?;
    let Listener::NativeTcp(options) = prepared.declaration() else {
        return Err("wrong listener provider".into());
    };
    let (stream, _) = tokio::time::timeout(STARTUP, listener.accept()).await??;
    let remote = carrier.dialer;
    Link::adopt(stream, carrier, remote, &options.socket, &options.tcp)
}

pub async fn dial(configuration: &NodeNetmap, carrier: Carrier) -> Result<Link> {
    let key = format!("{}/{}", carrier.listener, carrier.listener_key);
    let candidate = configuration
        .dialers
        .get(&key)
        .ok_or("missing actual dial candidate")?;
    let DialParameters::NativeTcp { options } = &candidate.parameters else {
        return Err("wrong dial provider".into());
    };
    if candidate.peer != carrier.listener
        || candidate.endpoints.len() != 1
        || !options.bind_interface.is_empty()
        || !options.local_address.is_empty()
        || !options.enabled
    {
        return Err("unexpected candidate endpoints".into());
    }
    let endpoint = &candidate.endpoints[0];
    let stream = tokio::time::timeout(STARTUP, async {
        loop {
            // Docker DNS resolves the underlay before any TUN route exists.
            let addresses =
                tokio::net::lookup_host((endpoint.host.as_str(), endpoint.port.get())).await?;
            for address in addresses.filter(SocketAddr::is_ipv4) {
                if let Ok(Ok(stream)) = tokio::time::timeout(
                    options.connect_timeout.as_std(),
                    TcpStream::connect(address),
                )
                .await
                {
                    return Ok::<_, crate::Error>(stream);
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    })
    .await??;
    let remote = carrier.listener;
    Link::adopt(stream, carrier, remote, &options.socket, &options.tcp)
}

pub fn inspect(frame: meshway_transport::Frame, link: &Link) -> Result<Vec<u8>> {
    if frame.peer != Some(link.peer) {
        return Err("actual socket peer mismatch".into());
    }
    for sentinel in crate::application::SENTINELS {
        if frame
            .payload
            .windows(sentinel.len())
            .any(|part| part == sentinel)
        {
            return Err("plaintext application sentinel exposed on carrier".into());
        }
    }
    Ok(frame.payload.into_vec())
}
