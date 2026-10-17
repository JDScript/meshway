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
    socket: std::io::Result<Box<dyn crate::host::Icmp>>,
) {
    let socket = match socket {
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
