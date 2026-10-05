use crate::{Arguments, Error};
use meshway_transport::{Completion, Frame, Receiver, SendContext, SendOutcome, Sender, Session};
use std::{
    collections::VecDeque,
    net::SocketAddr,
    time::{Duration, Instant},
};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

pub(super) struct Framed {
    sender: Sender,
    receiver: Receiver,
    destination: Option<SocketAddr>,
    expected_source: SocketAddr,
    peer: Option<JoinHandle<Result<(), Error>>>,
    stop: CancellationToken,
}

fn submit(sender: &mut Sender, mut frames: VecDeque<Frame>) -> Result<Completion, Error> {
    let count = frames.len();
    let admission = sender.try_send(&mut frames, SendContext::default());
    if admission.accepted != count || admission.stop.is_some() || !frames.is_empty() {
        return Err(format!("measurement batch admission failed: {:?}", admission.stop).into());
    }
    admission
        .completion
        .ok_or_else(|| "missing batch completion".into())
}

async fn complete(completion: Completion) -> Result<(), Error> {
    let completed = completion.wait().await?;
    if completed
        .frames
        .iter()
        .any(|frame| !matches!(frame.outcome, SendOutcome::LocalComplete))
    {
        return Err("transport batch did not complete locally".into());
    }
    // Original input allocations return through the production owner and are
    // released here. The benchmark never fabricates an I/O completion.
    Ok(())
}

impl Framed {
    pub(super) fn new(
        client: Session,
        server: Session,
        destination: Option<SocketAddr>,
        expected_source: SocketAddr,
        args: &Arguments,
    ) -> Self {
        let (sender, receiver) = client.into_split();
        let (mut peer_sender, mut peer_receiver) = server.into_split();
        let stop = CancellationToken::new();
        let cancelled = stop.clone();
        let batch_size = args.batch_size;
        let counts = [args.warmup, args.messages];
        let peer = tokio::spawn(async move {
            let result = tokio::select! { biased;
                _ = cancelled.cancelled() => Ok(()),
                result = async {
                    for mut remaining in counts {
                        while remaining > 0 {
                            let count = remaining.min(batch_size);
                            let mut frames = VecDeque::with_capacity(count as usize);
                            for _ in 0..count { frames.push_back(peer_receiver.recv().await?); }
                            complete(submit(&mut peer_sender, frames)?).await?;
                            remaining -= count;
                        }
                    }
                    Ok::<_, Error>(())
                } => result,
            };
            peer_sender.shutdown().await?;
            result
        });
        Self {
            sender,
            receiver,
            destination,
            expected_source,
            peer: Some(peer),
            stop,
        }
    }

    pub(super) async fn exchange(
        &mut self,
        packets: Vec<Vec<u8>>,
    ) -> Result<Vec<(Vec<u8>, Duration)>, Error> {
        let count = packets.len();
        let frames = packets
            .into_iter()
            .map(|bytes| Frame::new(bytes, self.destination))
            .collect();
        let start = Instant::now();
        let completed = submit(&mut self.sender, frames)?;
        let received = async {
            let mut echoed = Vec::with_capacity(count);
            for _ in 0..count {
                let frame = self.receiver.recv().await?;
                if frame.peer != Some(self.expected_source) {
                    return Err("echo source differs".into());
                }
                echoed.push((frame.payload, start.elapsed()));
            }
            Ok::<_, Error>(echoed)
        };
        let (_, echoed) = tokio::try_join!(complete(completed), received)?;
        Ok(echoed)
    }

    pub(super) async fn close(&mut self) -> Result<(), Error> {
        self.stop.cancel();
        self.sender.shutdown().await?;
        super::join_peer(&mut self.peer).await
    }
}

impl Drop for Framed {
    fn drop(&mut self) {
        self.stop.cancel();
        if let Some(peer) = &self.peer {
            peer.abort();
        }
    }
}
