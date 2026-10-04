//! Final I/O adapters over host-opened transport sessions and TUN devices.
//! These share the circuit/packet's original guard and spawn no tasks. Local
//! write completion is distinct from authenticated reception/application echo.
use crate::{Hop, PendingAction, secure::PendingFrame};
use meshway_transport::{AdmissionStop, Frame, SendContext, SendOutcome, Sender};
use std::{
    collections::VecDeque,
    fmt,
    future::{Future, poll_fn},
    net::SocketAddr,
    pin::Pin,
    sync::Arc,
    task::Poll,
};

impl fmt::Debug for crate::secure::Guard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PacketLifetime").finish_non_exhaustive()
    }
}

impl meshway_transport::SendLifetime for crate::secure::Guard {
    fn is_current(&self) -> bool {
        self.ensure_current().is_ok()
    }
    fn invalidated(self: Arc<Self>) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(async move { crate::secure::Guard::invalidated(&self).await })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("packet I/O was cancelled or its authority expired")]
    Cancelled,
    #[error("transport rejected packet admission: {0:?}")]
    Admission(AdmissionStop),
    #[error("transport completion was lost; delivery is unknown")]
    CompletionLost,
    #[error("transport did not complete the write; delivery may be partial")]
    Incomplete,
    #[error("packet output is not a local TUN delivery")]
    Destination,
    #[error("TUN packet write failed")]
    Tun,
}

pub async fn send(
    sender: &mut Sender,
    pending: PendingFrame,
    peer: Option<SocketAddr>,
) -> Result<(), Error> {
    let guard = pending.io_guard();
    let cancellation = guard.cancellation();
    // Cancelling this future also cancels any already-admitted transport work;
    // dropping a transport completion waiter alone intentionally would not.
    let _cancel_on_drop = cancellation.clone().drop_guard();
    let context = SendContext {
        cancellation: cancellation.clone(),
        deadline: Some(guard.expires_at().into()),
        lifetime: Some(Arc::new(guard.clone())),
    };
    let bytes = pending
        .release(|bytes| bytes)
        .map_err(|_| Error::Cancelled)?;
    let capacity = bytes.capacity();
    let mut frames = VecDeque::from([Frame::new(bytes, peer)]);
    let completion = loop {
        guard.ensure_current().map_err(|_| Error::Cancelled)?;
        let admitted = sender.try_send(&mut frames, context.clone());
        if admitted.accepted == 1 {
            break admitted.completion.ok_or(Error::CompletionLost)?;
        }
        match admitted.stop {
            Some(
                AdmissionStop::ByteBudget
                | AdmissionStop::FrameBudget
                | AdmissionStop::OperationBudget,
            ) => {
                tokio::select! { biased;
                    _ = guard.invalidated() => return Err(Error::Cancelled),
                    result = sender.wait_capacity(capacity) => result.map_err(Error::Admission)?,
                }
            }
            Some(reason) => return Err(Error::Admission(reason)),
            None => return Err(Error::CompletionLost),
        }
    };
    let completed = tokio::select! { biased;
        _ = guard.invalidated() => {
            cancellation.cancel();
            return Err(Error::Cancelled);
        },
        result = completion.wait() => result.map_err(|_| Error::CompletionLost)?,
    };
    if completed.frames.len() != 1
        || !completed
            .frames
            .iter()
            .all(|frame| matches!(frame.outcome, SendOutcome::LocalComplete))
    {
        return Err(Error::Incomplete);
    }
    guard.ensure_current().map_err(|_| Error::Cancelled)?;
    Ok(())
}

/// Keep the plaintext packet and all circuit/flow/lease guards owned throughout
/// the asynchronous write. Every resumed device poll rechecks those guards.
/// Exit/NAT forwarding has a separate owner and cannot use local TUN delivery.
pub async fn deliver(tun: &meshway_tun::Tun, pending: PendingAction) -> Result<(), Error> {
    if pending.hop() != Hop::Deliver {
        return Err(Error::Destination);
    }
    pending.ensure_current().map_err(|_| Error::Cancelled)?;
    let write = tun.send(pending.packet.as_bytes());
    tokio::pin!(write);
    tokio::select! { biased;
        _ = pending.invalidated() => Err(Error::Cancelled),
        result = poll_fn(|cx| {
            if pending.ensure_current().is_err() { return Poll::Ready(Err(Error::Cancelled)); }
            write.as_mut().poll(cx).map(|result| result.map_err(|_| Error::Tun))
        }) => result,
    }
}
