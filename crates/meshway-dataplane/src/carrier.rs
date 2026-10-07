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

#[derive(Clone, Copy, Debug)]
pub struct SendReceipt {
    /// Monotonic instant immediately before the successful transport admission,
    /// after capacity/lock waits. A receipt exists only after LocalComplete;
    /// it does not assert peer reception or a successful diagnostic reply.
    pub admitted_at: std::time::Instant,
}

pub async fn send(
    sender: &mut Sender,
    pending: PendingFrame,
    peer: Option<SocketAddr>,
) -> Result<(), Error> {
    send_scoped(sender, pending, peer, None).await
}

/// A channel owner can only restrict the original packet authority. Both
/// lifetimes are retained by accepted transport work, even if its completion
/// waiter remains alive but is no longer polled.
pub async fn send_scoped(
    sender: &mut Sender,
    pending: PendingFrame,
    peer: Option<SocketAddr>,
    channel: Option<Arc<dyn meshway_transport::SendLifetime>>,
) -> Result<(), Error> {
    send_scoped_with_receipt(sender, pending, peer, channel)
        .await
        .map(|_| ())
}

pub async fn send_scoped_with_receipt(
    sender: &mut Sender,
    pending: PendingFrame,
    peer: Option<SocketAddr>,
    channel: Option<Arc<dyn meshway_transport::SendLifetime>>,
) -> Result<SendReceipt, Error> {
    send_inner(SendOwner::Borrowed(sender), pending, peer, channel).await
}

/// Shared endpoint admission never holds its mutex while awaiting completion.
/// One peer's unpolled caller must not retain another peer's physical sender.
pub async fn send_shared(
    sender: &tokio::sync::Mutex<Sender>,
    pending: PendingFrame,
    peer: SocketAddr,
    channel: Arc<dyn meshway_transport::SendLifetime>,
) -> Result<(), Error> {
    send_shared_with_receipt(sender, pending, peer, channel)
        .await
        .map(|_| ())
}

pub async fn send_shared_with_receipt(
    sender: &tokio::sync::Mutex<Sender>,
    pending: PendingFrame,
    peer: SocketAddr,
    channel: Arc<dyn meshway_transport::SendLifetime>,
) -> Result<SendReceipt, Error> {
    send_inner(
        SendOwner::Shared(sender),
        pending,
        Some(peer),
        Some(channel),
    )
    .await
}

enum SendOwner<'a> {
    Borrowed(&'a mut Sender),
    Shared(&'a tokio::sync::Mutex<Sender>),
}

impl SendOwner<'_> {
    async fn admit(
        &mut self,
        frames: &mut VecDeque<Frame>,
        context: SendContext,
    ) -> (meshway_transport::Admission, std::time::Instant) {
        match self {
            Self::Borrowed(sender) => {
                let admitted_at = std::time::Instant::now();
                (sender.try_send(frames, context), admitted_at)
            }
            Self::Shared(sender) => {
                let mut sender = sender.lock().await;
                let admitted_at = std::time::Instant::now();
                (sender.try_send(frames, context), admitted_at)
            }
        }
    }
    async fn capacity(&self, capacity: usize) -> Result<(), Error> {
        match self {
            Self::Borrowed(sender) => sender
                .wait_capacity(capacity)
                .await
                .map_err(Error::Admission),
            Self::Shared(_) => {
                // No borrowed wait_capacity future may retain the shared sender
                // lock. Admission rechecks all budgets after this paced retry.
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
                Ok(())
            }
        }
    }
}

async fn send_inner(
    sender: SendOwner<'_>,
    pending: PendingFrame,
    peer: Option<SocketAddr>,
    channel: Option<Arc<dyn meshway_transport::SendLifetime>>,
) -> Result<SendReceipt, Error> {
    static PROFILE: meshway_logging::profile::Metric =
        meshway_logging::profile::Metric::new("carrier.send.poll");
    PROFILE
        .poll(send_owned(sender, pending, peer, channel))
        .await
}

async fn send_owned(
    mut sender: SendOwner<'_>,
    pending: PendingFrame,
    peer: Option<SocketAddr>,
    channel: Option<Arc<dyn meshway_transport::SendLifetime>>,
) -> Result<SendReceipt, Error> {
    use meshway_transport::SendLifetime;
    let guard = pending.io_guard();
    let lifetime = Arc::new(OutputLifetime {
        packet: guard.clone(),
        channel,
    });
    let cancellation = guard.cancellation();
    // Cancelling this future also cancels any already-admitted transport work;
    // dropping a transport completion waiter alone intentionally would not.
    let _cancel_on_drop = cancellation.clone().drop_guard();
    let context = SendContext {
        cancellation: cancellation.clone(),
        deadline: Some(guard.expires_at().into()),
        lifetime: Some(lifetime.clone()),
    };
    let bytes = pending
        .release(|bytes| bytes.into_vec())
        .map_err(|_| Error::Cancelled)?;
    let capacity = bytes.capacity();
    let mut frames = VecDeque::from([Frame::new(bytes, peer)]);
    let (completion, admitted_at) = loop {
        let (admitted, admitted_at) =
            guarded(&lifetime, sender.admit(&mut frames, context.clone())).await?;
        if admitted.accepted == 1 {
            break (
                admitted.completion.ok_or(Error::CompletionLost)?,
                admitted_at,
            );
        }
        match admitted.stop {
            Some(
                AdmissionStop::ByteBudget
                | AdmissionStop::FrameBudget
                | AdmissionStop::OperationBudget,
            ) => {
                guarded(&lifetime, sender.capacity(capacity)).await??;
            }
            Some(reason) => return Err(Error::Admission(reason)),
            None => return Err(Error::CompletionLost),
        }
    };
    let completed = guarded(&lifetime, completion.wait())
        .await?
        .map_err(|_| Error::CompletionLost)?;
    if completed.frames.len() != 1
        || !completed
            .frames
            .iter()
            .all(|frame| matches!(frame.outcome, SendOutcome::LocalComplete))
    {
        return Err(Error::Incomplete);
    }
    if !lifetime.is_current() {
        return Err(Error::Cancelled);
    }
    Ok(SendReceipt { admitted_at })
}

/// Ready admission/completion needs the same pre/post fences, but no allocated
/// cancellation waiter. Pending work is polled once per outer poll; its original
/// lifetime is then registered and retained. Accepted transport work separately
/// owns that lifetime, even while this caller is alive but no longer polled.
async fn guarded<T>(
    lifetime: &Arc<OutputLifetime>,
    future: impl Future<Output = T>,
) -> Result<T, Error> {
    use meshway_transport::SendLifetime;
    tokio::pin!(future);
    let mut invalidated = None;
    poll_fn(|cx| {
        if !lifetime.is_current() {
            return Poll::Ready(Err(Error::Cancelled));
        }
        if let Poll::Ready(value) = future.as_mut().poll(cx) {
            return Poll::Ready(if lifetime.is_current() {
                Ok(value)
            } else {
                Err(Error::Cancelled)
            });
        }
        let wait = invalidated.get_or_insert_with(|| lifetime.clone().invalidated());
        if wait.as_mut().poll(cx).is_ready() {
            Poll::Ready(Err(Error::Cancelled))
        } else {
            Poll::Pending
        }
    })
    .await
}

#[derive(Debug)]
struct OutputLifetime {
    packet: crate::secure::Guard,
    channel: Option<Arc<dyn meshway_transport::SendLifetime>>,
}

impl meshway_transport::SendLifetime for OutputLifetime {
    fn is_current(&self) -> bool {
        self.packet.ensure_current().is_ok()
            && self
                .channel
                .as_ref()
                .is_none_or(|channel| channel.is_current())
    }

    fn invalidated(self: Arc<Self>) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(async move {
            tokio::select! { biased;
                _ = self.packet.invalidated() => {},
                _ = async {
                    match &self.channel {
                        Some(channel) => channel.clone().invalidated().await,
                        None => std::future::pending().await,
                    }
                } => {},
            }
        })
    }
}

/// Keep the plaintext packet and all circuit/flow/lease guards owned throughout
/// the asynchronous write. Every resumed device poll rechecks those guards.
/// Exit/NAT forwarding has a separate owner and cannot use local TUN delivery.
pub async fn deliver(tun: &meshway_tun::Tun, pending: PendingAction) -> Result<(), Error> {
    static PROFILE: meshway_logging::profile::Metric =
        meshway_logging::profile::Metric::new("carrier.deliver.poll");
    PROFILE.poll(deliver_owned(tun, pending)).await
}

async fn deliver_owned(tun: &meshway_tun::Tun, pending: PendingAction) -> Result<(), Error> {
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
