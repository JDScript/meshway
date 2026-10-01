//! Readiness awaits own no answer bytes. A read lock covers fresh TTL encoding
//! and the nonblocking syscall, linearized against every publication mutation.

use std::{future::Future, io, net::SocketAddr, sync::Arc, time::Duration};
use tokio::{
    net::{TcpStream, UdpSocket},
    time::{Instant, sleep_until},
};

use super::{Guard, Response, encode_bounded};

const SEND_TIMEOUT: Duration = Duration::from_secs(1);

trait Writer {
    fn writable(&self) -> impl Future<Output = io::Result<()>> + Send;
    /// Must never block or wait; called while holding the publication read lock.
    fn try_write(&self, bytes: &[u8]) -> io::Result<usize>;
}

struct Datagram<'a> {
    socket: &'a UdpSocket,
    peer: SocketAddr,
}

impl Writer for Datagram<'_> {
    async fn writable(&self) -> io::Result<()> {
        self.socket.writable().await
    }
    fn try_write(&self, bytes: &[u8]) -> io::Result<usize> {
        self.socket.try_send_to(bytes, self.peer)
    }
}

impl Writer for TcpStream {
    async fn writable(&self) -> io::Result<()> {
        TcpStream::writable(self).await
    }
    fn try_write(&self, bytes: &[u8]) -> io::Result<usize> {
        TcpStream::try_write(self, bytes)
    }
}

impl Guard {
    fn current(&self, response: &Response, now: Instant) -> bool {
        let state = self.0.read().expect("DNS guard poisoned");
        Arc::ptr_eq(&response.generation, &state.generation)
            && response
                .deadline
                .is_none_or(|deadline| deadline.saturating_duration_since(now).as_secs() > 0)
    }

    /// None = withdrawn/expired; WouldBlock = discard bytes and await readiness
    /// again; any partial TCP frame = close without ever completing the old body.
    fn try_send(
        &self,
        response: &Response,
        writer: &impl Writer,
        clock: &impl Fn() -> Instant,
    ) -> io::Result<Option<()>> {
        let state = self.0.read().expect("DNS guard poisoned");
        if !Arc::ptr_eq(&response.generation, &state.generation) {
            return Ok(None);
        }
        // Encoding is bounded, but a seconds boundary may pass even without an
        // await. Regenerate once in that case; never transmit an overstated TTL.
        for _ in 0..2 {
            let now = clock();
            let remaining = response
                .deadline
                .map(|deadline| deadline.saturating_duration_since(now).as_secs());
            if remaining == Some(0) {
                return Ok(None);
            }
            let Some(mut bytes) = encode_bounded(
                Self::answer_locked(&state, &response.query, now),
                response.limit,
            ) else {
                return Ok(None);
            };
            if response.tcp {
                let prefix = (bytes.len() as u16).to_be_bytes();
                bytes.splice(..0, prefix);
            }
            if response
                .deadline
                .map(|deadline| deadline.saturating_duration_since(clock()).as_secs())
                != remaining
            {
                continue;
            }
            let sent = writer.try_write(&bytes)?;
            return if sent == bytes.len() {
                Ok(Some(()))
            } else {
                Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "partial DNS frame; closing connection",
                ))
            };
        }
        Ok(None)
    }
}

async fn transmit(
    writer: &impl Writer,
    guard: &Guard,
    response: Response,
    clock: impl Fn() -> Instant,
) -> io::Result<()> {
    let mut changes = guard.1.subscribe();
    let timeout = Instant::now() + SEND_TIMEOUT;
    // Positive TTLs must cease when less than one whole second remains. Queries
    // without a publication deadline are still bounded by the send timeout.
    let lease = response
        .deadline
        .map(|deadline| deadline - Duration::from_secs(1))
        .unwrap_or(timeout);
    loop {
        if !guard.current(&response, clock()) {
            return Ok(());
        }
        tokio::select! {
            biased;
            _ = changes.changed() => return Ok(()),
            _ = sleep_until(lease.min(timeout)) => return Ok(()),
            result = writer.writable() => result?,
        }
        // Also enforce the absolute cutoff directly: a delayed task/IO driver
        // must not let stale readiness win over an elapsed timer registration.
        if Instant::now() >= lease.min(timeout) {
            return Ok(());
        }
        match guard.try_send(&response, writer, &clock) {
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => continue,
            result => return result.map(|_| ()),
        }
    }
}

pub(super) async fn udp(
    socket: &UdpSocket,
    guard: &Guard,
    request: &[u8],
    peer: SocketAddr,
) -> io::Result<()> {
    if let Some(response) = guard.prepare(request, false) {
        transmit(&Datagram { socket, peer }, guard, response, Instant::now).await?;
    }
    Ok(())
}

pub(super) async fn tcp(stream: &TcpStream, guard: &Guard, request: &[u8]) -> io::Result<()> {
    if let Some(response) = guard.prepare(request, true) {
        transmit(stream, guard, response, Instant::now).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
