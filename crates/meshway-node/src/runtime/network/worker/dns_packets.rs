//! Local DNS packets share the worker's bounded device turn, not Mesh flows.
use std::{
    future::{Future, poll_fn},
    sync::Arc,
};

pub(super) async fn receive(
    port: &Option<Arc<meshway_dns::packet::Port>>,
) -> Option<meshway_dns::packet::Output> {
    match port {
        Some(port) => port.recv().await,
        None => std::future::pending().await,
    }
}
pub(super) async fn send(
    host: &dyn crate::host::Host,
    output: Option<&meshway_dns::packet::Output>,
) -> Result<(), meshway_tun::Error> {
    let Some(output) = output else {
        return std::future::pending().await;
    };
    let Some(device) = host.tun() else {
        return Err(meshway_tun::Error::Unsupported);
    };
    let sending = device.send(output.bytes());
    tokio::pin!(sending);
    // One datagram: a Pending write committed nothing. Recreating it on the
    // next worker turn is safe; every poll still checks the original DNS guard.
    tokio::select! { biased;
        _ = output.ended() => Ok(()),
        result = poll_fn(|cx| {
            if output.ensure_current().is_err() { return std::task::Poll::Ready(Ok(())); }
            sending.as_mut().poll(cx)
        }) => result,
    }
}
