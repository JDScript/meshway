mod framed;
mod tcp;
mod udp;

use crate::{Arguments, Error, Mode, Protocol};
use meshway_transport::Limits;
use meshway_transport_spec::{SocketOptions, TcpOptions};
use meshway_transport_tcp::TcpTransport;
use meshway_transport_udp::UdpTransport;
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream, UdpSocket};

enum Backend {
    RawTcp(tcp::RawTcp<TcpTransport>),
    RawUdp(udp::RawUdp),
    Framed(framed::Framed),
}

pub(crate) struct Connection(Backend);

impl Connection {
    pub(crate) async fn open(args: &Arguments) -> Result<Self, Error> {
        Self::open_backend(args).await.map(Self)
    }

    async fn open_backend(args: &Arguments) -> Result<Backend, Error> {
        let socket = SocketOptions {
            receive_buffer: 1024 * 1024,
            send_buffer: 1024 * 1024,
        };
        let frames = args.batch_size as usize * 2;
        let limits = Limits::new(
            args.payload_bytes as usize,
            frames,
            args.payload_bytes as usize * frames,
            2,
            args.batch_size as usize,
        )?;
        match args.protocol {
            Protocol::Tcp => {
                let listener = TcpListener::bind("127.0.0.1:0").await?;
                let client = TcpStream::connect(listener.local_addr()?).await?;
                let (server, _) = listener.accept().await?;
                let client = TcpTransport::adopt(client, &socket, &TcpOptions::default())?;
                let server = TcpTransport::adopt(server, &socket, &TcpOptions::default())?;
                let peer = server.local_addr()?;
                match args.mode {
                    Mode::Raw => Ok(Backend::RawTcp(tcp::RawTcp::new(
                        client,
                        server,
                        args,
                        limits.stream_buffer_bytes().min(64 * 1024),
                    ))),
                    Mode::Datagram => Ok(Backend::Framed(framed::Framed::new(
                        client.into_session(limits)?,
                        server.into_session(limits)?,
                        None,
                        peer,
                        args,
                    ))),
                }
            }
            Protocol::Udp => {
                let client = UdpTransport::adopt(UdpSocket::bind("127.0.0.1:0").await?, &socket)?;
                let server = UdpTransport::adopt(UdpSocket::bind("127.0.0.1:0").await?, &socket)?;
                let peer = server.local_addr()?;
                match args.mode {
                    Mode::Raw => Ok(Backend::RawUdp(udp::RawUdp::new(client, server, args)?)),
                    Mode::Datagram => Ok(Backend::Framed(framed::Framed::new(
                        client.into_session(limits),
                        server.into_session(limits),
                        Some(peer),
                        peer,
                        args,
                    ))),
                }
            }
        }
    }

    pub(crate) async fn exchange(
        &mut self,
        packets: Vec<Vec<u8>>,
    ) -> Result<Vec<(Vec<u8>, Duration)>, Error> {
        match &mut self.0 {
            Backend::RawTcp(connection) => connection.exchange(packets).await,
            Backend::RawUdp(connection) => connection.exchange(packets).await,
            Backend::Framed(connection) => connection.exchange(packets).await,
        }
    }

    pub(crate) async fn close(&mut self) -> Result<(), Error> {
        match &mut self.0 {
            Backend::RawTcp(connection) => connection.close().await,
            Backend::RawUdp(connection) => connection.close().await,
            Backend::Framed(connection) => connection.close().await,
        }
    }
}
