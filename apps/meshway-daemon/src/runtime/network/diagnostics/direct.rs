//! Echo from this host's own network for a direct or unmanaged decision: a
//! raw ICMP socket without the protection mark, so the host routes it as any
//! application's packet. It carries no Mesh authority and touches no Mesh state.
use super::external::{Counts, report};
use crate::runtime::PingRequest;
use crate::runtime::ping::Progress;
use meshway_dataplane::diagnostics::echo::Echo;
use meshway_protocol::ipc::{PingError, PingEvent};
use std::{net::Ipv4Addr, time::Instant};
use tokio::sync::watch;

pub(super) async fn run(
    target: Ipv4Addr,
    identifier: u16,
    request: PingRequest,
    _progress: watch::Sender<Progress>,
) {
    let socket = match Socket::open() {
        Ok(socket) => socket,
        Err(error) => {
            tracing::warn!(%error, "Direct echo socket unavailable");
            let _ = request.events.try_send(PingEvent::Failed {
                code: PingError::Unsupported,
            });
            request.cancellation.cancel();
            return;
        }
    };
    let mut echo = Echo::new(target, identifier, Instant::now());
    let mut counts = Counts::default();
    let mut buffer = vec![0u8; 2048];
    while counts.results < request.count && !request.cancellation.is_cancelled() {
        tokio::select! { biased;
            _ = request.cancellation.cancelled() => return,
            _ = tokio::time::sleep_until(echo.wake_at().into()) => {}
        }
        let now = Instant::now();
        if let Some(result) = echo.expire(now) {
            let event = counts.record(&result, Vec::new());
            report(&request, &mut counts, event);
            continue;
        }
        if !echo.due(now) {
            continue;
        }
        let (_, message) = echo.message(now);
        match socket.send(target, &message).await {
            Ok(()) => echo.admitted(Instant::now(), false),
            Err(error) => {
                tracing::debug!(%error, "Direct echo send failed");
                if let Some(result) = echo.refused(Instant::now()) {
                    let event = counts.record(&result, Vec::new());
                    report(&request, &mut counts, event);
                }
                continue;
            }
        }
        // Read until this request is answered or its deadline passes.
        loop {
            let received = tokio::select! { biased;
                _ = request.cancellation.cancelled() => return,
                _ = tokio::time::sleep_until(echo.wake_at().into()) => break,
                received = socket.recv(&mut buffer) => received,
            };
            let Ok(length) = received else { break };
            let at = Instant::now();
            let Ok(packet) = meshway_dataplane::Packet::parse(&buffer[..length]) else {
                continue;
            };
            if let Some(result) = echo.answer(&packet.facts(), at) {
                let event = counts.record(&result, Vec::new());
                report(&request, &mut counts, event);
                break;
            }
        }
    }
}

#[cfg(target_os = "linux")]
struct Socket(tokio::io::unix::AsyncFd<std::os::fd::OwnedFd>);

#[cfg(target_os = "linux")]
impl Socket {
    fn open() -> std::io::Result<Self> {
        use nix::sys::socket::{AddressFamily, SockFlag, SockProtocol, SockType, socket};
        let fd = socket(
            AddressFamily::Inet,
            SockType::Raw,
            SockFlag::SOCK_NONBLOCK | SockFlag::SOCK_CLOEXEC,
            SockProtocol::Icmp,
        )?;
        Ok(Self(tokio::io::unix::AsyncFd::new(fd)?))
    }

    async fn send(&self, target: Ipv4Addr, message: &[u8]) -> std::io::Result<()> {
        use nix::sys::socket::{MsgFlags, SockaddrIn, sendto};
        use std::os::fd::AsRawFd;
        let address = SockaddrIn::from(std::net::SocketAddrV4::new(target, 0));
        loop {
            let mut ready = self.0.writable().await?;
            match ready.try_io(|fd| {
                sendto(fd.as_raw_fd(), message, &address, MsgFlags::empty())
                    .map_err(std::io::Error::from)
            }) {
                Ok(result) => return result.map(|_| ()),
                Err(_would_block) => continue,
            }
        }
    }

    /// One received IPv4 packet, header included.
    async fn recv(&self, buffer: &mut [u8]) -> std::io::Result<usize> {
        use nix::sys::socket::{MsgFlags, recv};
        use std::os::fd::AsRawFd;
        loop {
            let mut ready = self.0.readable().await?;
            match ready.try_io(|fd| {
                recv(fd.as_raw_fd(), buffer, MsgFlags::empty()).map_err(std::io::Error::from)
            }) {
                Ok(result) => return result,
                Err(_would_block) => continue,
            }
        }
    }
}

/// Raw ICMP echo is implemented for the Linux runtime only.
#[cfg(not(target_os = "linux"))]
struct Socket;

#[cfg(not(target_os = "linux"))]
impl Socket {
    fn open() -> std::io::Result<Self> {
        Err(std::io::ErrorKind::Unsupported.into())
    }
    async fn send(&self, _: Ipv4Addr, _: &[u8]) -> std::io::Result<()> {
        Err(std::io::ErrorKind::Unsupported.into())
    }
    async fn recv(&self, _: &mut [u8]) -> std::io::Result<usize> {
        Err(std::io::ErrorKind::Unsupported.into())
    }
}
