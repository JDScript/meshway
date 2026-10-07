use super::*;
use crate::secure::pipeline::timing;
use meshway_transport::{BatchOutcome, ChannelScope, ScopedFrame, SendLifetime};

pub async fn send_batch(
    sender: &mut Sender,
    frames: Vec<PendingFrame>,
    peer: Option<SocketAddr>,
    scope: ChannelScope,
    channel: Option<Arc<dyn SendLifetime>>,
) -> Result<SendReceipt, Error> {
    send(SendOwner::Borrowed(sender), frames, peer, scope, channel).await
}

pub async fn send_batch_shared(
    sender: &tokio::sync::Mutex<Sender>,
    frames: Vec<PendingFrame>,
    peer: SocketAddr,
    scope: ChannelScope,
    channel: Arc<dyn SendLifetime>,
) -> Result<SendReceipt, Error> {
    send(
        SendOwner::Shared(sender),
        frames,
        Some(peer),
        scope,
        Some(channel),
    )
    .await
}

async fn send(
    mut sender: SendOwner<'_>,
    pending: Vec<PendingFrame>,
    peer: Option<SocketAddr>,
    scope: ChannelScope,
    channel: Option<Arc<dyn SendLifetime>>,
) -> Result<SendReceipt, Error> {
    static PROFILE: meshway_logging::profile::Metric =
        meshway_logging::profile::Metric::new("carrier.send.poll");
    PROFILE
        .poll(async {
            if pending.is_empty() || pending.len() > 32 {
                return Err(Error::Admission(AdmissionStop::BatchLimit));
            }
            // Control frames cannot accidentally enter the business batch API.
            if pending.iter().any(|frame| !frame.is_data()) {
                return Err(Error::Destination);
            }
            let lifetime = Arc::new(OutputLifetime {
                packets: pending.iter().map(PendingFrame::io_guard).collect(),
                channel,
            });
            // A legal jumbo stream frame can exceed Session's batch ceiling. Keep
            // the original batch intersection while using independent single-frame
            // operations, rather than waiting forever for impossible batch capacity.
            if pending.iter().any(|frame| frame.capacity() > 65536) {
                let mut first = None;
                for frame in pending {
                    let owner = match &mut sender {
                        SendOwner::Borrowed(sender) => SendOwner::Borrowed(sender),
                        SendOwner::Shared(sender) => SendOwner::Shared(sender),
                    };
                    let receipt =
                        super::send_owned(owner, frame, peer, Some(lifetime.clone())).await?;
                    first.get_or_insert(receipt);
                }
                return first.ok_or(Error::CompletionLost);
            }
            let cancellation = tokio_util::sync::CancellationToken::new();
            let _cancel = cancellation.clone().drop_guard();
            let context = SendContext {
                cancellation,
                deadline: lifetime
                    .packets
                    .iter()
                    .map(|guard| guard.expires_at().into())
                    .min(),
                lifetime: Some(lifetime.clone()),
            };
            let mut frames = VecDeque::with_capacity(pending.len());
            timing::batch("carrier.send.batch", pending.len());
            let released_at = timing::start();
            let buffers = PendingFrame::release_batch(pending).map_err(|_| Error::Cancelled)?;
            for (bytes, guard) in buffers.into_iter().zip(&lifetime.packets) {
                // Every original guard is retained by the common OutputLifetime;
                // the per-frame deadline also restricts Session's intersection.
                let deadline = Some(guard.expires_at().into());
                frames.push_back(ScopedFrame {
                    frame: Frame::owned(bytes, peer),
                    scope: scope.clone(),
                    context: SendContext {
                        deadline,
                        ..Default::default()
                    },
                });
            }
            let mut first_admission = None;
            while !frames.is_empty() {
                let (admitted, at) = match &mut sender {
                    SendOwner::Borrowed(sender) => {
                        let at = std::time::Instant::now();
                        (sender.submit_batch(&mut frames, context.clone()), at)
                    }
                    SendOwner::Shared(sender) => {
                        let mut sender = guarded(&lifetime, sender.lock()).await?;
                        let at = std::time::Instant::now();
                        (sender.submit_batch(&mut frames, context.clone()), at)
                    }
                };
                if admitted.accepted != 0 {
                    if first_admission.is_none()
                        && let Some(released_at) = released_at
                    {
                        static WALL: meshway_logging::profile::Metric =
                            meshway_logging::profile::Metric::new(
                                "carrier.release_to_admission.wall",
                            );
                        WALL.elapsed(at.duration_since(released_at));
                    }
                    first_admission.get_or_insert(at);
                    let completion = admitted.completion.ok_or(Error::CompletionLost)?;
                    let completed = guarded(&lifetime, completion.wait())
                        .await?
                        .map_err(|_| Error::CompletionLost)?;
                    if !matches!(completed.outcome(), BatchOutcome::LocalComplete) {
                        return Err(Error::Incomplete);
                    }
                    if released_at.is_some() {
                        static WALL: meshway_logging::profile::Metric =
                            meshway_logging::profile::Metric::new(
                                "carrier.admission_to_completion.wall",
                            );
                        WALL.elapsed(at.elapsed());
                    }
                    continue;
                }
                match admitted.stop {
                    Some(
                        AdmissionStop::ByteBudget
                        | AdmissionStop::FrameBudget
                        | AdmissionStop::OperationBudget,
                    ) => {
                        let capacity = frames.front().expect("nonempty").frame.payload.capacity();
                        guarded(&lifetime, sender.capacity(capacity)).await??;
                    }
                    Some(reason) => return Err(Error::Admission(reason)),
                    None => return Err(Error::CompletionLost),
                }
            }
            if !lifetime.is_current() {
                return Err(Error::Cancelled);
            }
            Ok(SendReceipt {
                admitted_at: first_admission.ok_or(Error::CompletionLost)?,
            })
        })
        .await
}

/// One intersection of original packet lifetimes for each TUN syscall. Partial
/// writes advance only the returned prefix; the suffix keeps every original cap.
pub async fn deliver_batch(
    tun: &meshway_tun::Tun,
    pending: Vec<PendingAction>,
) -> Result<(), Error> {
    static PROFILE: meshway_logging::profile::Metric =
        meshway_logging::profile::Metric::new("carrier.deliver.poll");
    PROFILE
        .poll(async {
            if pending.len() > 32 || pending.iter().any(|packet| packet.hop() != Hop::Deliver) {
                return Err(Error::Destination);
            }
            if pending.is_empty() {
                return Ok(());
            }
            let lifetime = Arc::new(OutputLifetime {
                packets: pending
                    .iter()
                    .map(|packet| crate::secure::Guard::from_route(&packet.pending))
                    .collect(),
                channel: None,
            });
            let slices: Vec<_> = pending
                .iter()
                .map(|packet| packet.packet.as_bytes())
                .collect();
            let mut offset = 0;
            while offset < pending.len() {
                let at = timing::start();
                let written = guarded(&lifetime, tun.send_batch(&slices[offset..]))
                    .await?
                    .map_err(|_| Error::Tun)?;
                if written == 0 || written > slices.len() - offset {
                    return Err(Error::Tun);
                }
                timing::batch("carrier.tun.written.batch", written);
                if let Some(at) = at {
                    static WALL: meshway_logging::profile::Metric =
                        meshway_logging::profile::Metric::new("carrier.tun.write.wall");
                    WALL.elapsed(at.elapsed());
                }
                offset += written;
            }
            Ok(())
        })
        .await
}
