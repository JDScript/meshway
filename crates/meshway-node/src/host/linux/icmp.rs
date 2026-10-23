use crate::host::{Icmp, LocalDiagnostics, SendFuture};
use std::net::Ipv4Addr;

pub(super) struct Diagnostics;
impl LocalDiagnostics for Diagnostics {
    fn direct_icmp(&self) -> std::io::Result<Box<dyn Icmp>> {
        Ok(Box::new(Socket::open()?))
    }
    fn resolve<'a>(
        &'a self,
        name: &'a str,
    ) -> SendFuture<'a, std::io::Result<Vec<std::net::IpAddr>>> {
        Box::pin(async move {
            Ok(tokio::net::lookup_host((name, 0))
                .await?
                .map(|address| address.ip())
                .collect())
        })
    }
}

struct Socket(tokio::io::unix::AsyncFd<std::os::fd::OwnedFd>);
impl Socket {
    fn open() -> std::io::Result<Self> {
        use nix::sys::socket::{AddressFamily, SockFlag, SockProtocol, SockType, socket};
        let fd = socket(
            AddressFamily::Inet,
            SockType::Raw,
            SockFlag::SOCK_NONBLOCK | SockFlag::SOCK_CLOEXEC,
            SockProtocol::Icmp,
        )?;
        // RUSTSEC-2026-0386: the AsyncFd owns this fd for life; nothing dups or closes it.
        #[allow(deprecated)]
        let fd = tokio::io::unix::AsyncFd::new(fd)?;
        Ok(Self(fd))
    }
}
impl Icmp for Socket {
    fn send<'a>(
        &'a self,
        target: Ipv4Addr,
        message: &'a [u8],
    ) -> SendFuture<'a, std::io::Result<()>> {
        Box::pin(async move {
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
        })
    }
    fn recv<'a>(&'a self, buffer: &'a mut [u8]) -> SendFuture<'a, std::io::Result<usize>> {
        Box::pin(async move {
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
        })
    }
}
