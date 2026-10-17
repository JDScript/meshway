//! The exit descriptor never leaves this bounded I/O owner.
use super::tuple::OriginalTuple;
use crate::{Error, Result};
use meshway_model::Digest;
use meshway_tun::Tun;
use std::{
    collections::BTreeMap,
    sync::{Arc, RwLock},
    time::Instant,
};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

/// The absolute monotonic and UTC cutoff of one admitted projection, both
/// exclusive. It is the packet owner's own bound, never a lease.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExitCutoff {
    pub at: Instant,
    pub ms: u64,
}
/// The current monotonic and UTC clocks, read once for several checks.
#[derive(Clone, Copy)]
pub(super) struct Now {
    at: Instant,
    ms: Option<u64>,
}
impl Now {
    pub fn read() -> Self {
        Self {
            at: Instant::now(),
            ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .ok()
                .and_then(|elapsed| u64::try_from(elapsed.as_millis()).ok()),
        }
    }
}

impl ExitCutoff {
    pub fn valid(&self) -> bool {
        self.valid_at(Now::read())
    }
    fn valid_at(&self, now: Now) -> bool {
        now.at < self.at && now.ms.is_some_and(|ms| ms < self.ms)
    }
    pub fn expires_at(&self) -> Instant {
        self.at
    }
    pub fn expires_at_ms(&self) -> u64 {
        self.ms
    }
    pub async fn ended(&self, cancelled: &CancellationToken) {
        tokio::select! { biased;
            _ = cancelled.cancelled() => {},
            _ = tokio::time::sleep_until(self.at.into()) => {},
        }
    }
}

/// Opaque producer-owned authorization/path binding. Revocation is irreversible.
/// X4 maps the admitted data-plane lifetime, never observed CT/reply state.
#[derive(Clone)]
pub struct ExitScope {
    pub binding: Digest,
    pub revoked: CancellationToken,
}

#[derive(Clone)]
pub(super) struct Live {
    pub tuple: OriginalTuple,
    pub deadline: ExitCutoff,
    pub scope: ExitScope,
    pub retired: CancellationToken,
    pub gate_until: Instant,
}

impl Live {
    pub fn valid(&self) -> bool {
        self.valid_at(Now::read())
    }
    pub fn valid_at(&self, now: Now) -> bool {
        self.deadline.valid_at(now) && !self.ended() && now.at < self.gate_until
    }
    /// Retired or revoked; its cutoffs are checked with a clock.
    pub fn ended(&self) -> bool {
        self.scope.revoked.is_cancelled() || self.retired.is_cancelled()
    }
    /// The reverse-NAT reply of this projection or an ICMP error quoting its
    /// forward packet (conntrack RELATED); framing only, the flow owner admits.
    fn reply(&self, packet: &[u8]) -> bool {
        self.tuple.related(packet) || self.tuple.reply(packet)
    }
}

pub(super) type View = Arc<RwLock<BTreeMap<super::ExitIncarnation, Live>>>;

type Authority = super::super::super::protection::ProtectionScope;

/// One packet for the exit TUN. A write the caller awaits carries its reply
/// and the authority it was validated under; a queued write of an installed
/// projection carries neither: the owner validates the role's authority once
/// per batch and only counts what it cannot write.
struct Write {
    packet: Vec<u8>,
    reply: Option<oneshot::Sender<Result<()>>>,
    authority: Option<Authority>,
    live: Live,
    packet_guard: Box<dyn super::ExitSendGuard>,
}

/// Batches queued for the owner at most. The packet owner submits one batch
/// of at most 32 per turn, so about 1,024 packets (at 140,000 packets/s,
/// 7 ms) wait here at most: room for the bursts of its turns.
const BATCHES: usize = 32;
/// Writes one turn of the owner takes; adjacent compatible packets of a
/// turn share one TUN write (offload).
const WRITE_BATCH: usize = 64;

/// One packet of an installed projection, ready for [`super::ExitRole::submit_writes`].
pub struct ExitWrite(Write);

fn answer(write: Write, result: Result<()>) {
    if let Some(reply) = write.reply {
        let _ = reply.send(result);
    }
}

/// Whether every packet of the unwritten part of a turn is still writable:
/// its projection, its producer's guard, and the authority it is written
/// under (the turn's own once, a caller's per packet). One clock reading.
fn writable(writes: &[Write], current: Option<&Authority>, now: Now) -> bool {
    current.is_none_or(|authority| authority.check().is_ok())
        && writes.iter().all(|write| {
            write.live.valid_at(now)
                && write
                    .authority
                    .as_ref()
                    .is_none_or(|authority| authority.check().is_ok())
                && write.packet_guard.check().is_ok()
        })
}

/// Write one turn in order. A packet whose projection, guard or authority
/// ended is dropped before its write; a failed TUN write drops the first
/// packet it carried and the rest continue. Neither ends a flow here.
async fn write_batch(
    tun: &Tun,
    batch: Vec<Write>,
    current: Option<Authority>,
    stopping: &CancellationToken,
    failed: &mut u64,
) {
    let mut writes = Vec::with_capacity(batch.len());
    for write in batch {
        if (write.authority.is_some() || current.is_some())
            && write.live.tuple.matches(&write.packet)
        {
            writes.push(write);
        } else {
            answer(write, Err(Error::Expired));
        }
    }
    while !writes.is_empty() {
        // Every pending packet is checked before each poll of the write.
        let result = {
            let packets: Vec<&[u8]> = writes.iter().map(|write| write.packet.as_slice()).collect();
            let all = || {
                if writable(&writes, current.as_ref(), Now::read()) {
                    Ok(())
                } else {
                    Err(super::Invalidated)
                }
            };
            tokio::select! { biased;
                _ = stopping.cancelled() => Err(()),
                result = super::guard::checked(all, tun.send_batch(&packets)) => Ok(result.ok()),
            }
        };
        match result {
            Err(()) => {
                for write in writes.drain(..) {
                    answer(write, Err(Error::Expired));
                }
            }
            // Some packet stopped being writable: drop exactly those.
            Ok(None) => {
                let now = Now::read();
                let turn = current
                    .as_ref()
                    .is_none_or(|authority| authority.check().is_ok());
                let (keep, ended): (Vec<_>, Vec<_>) = writes
                    .drain(..)
                    .partition(|write| turn && writable(std::slice::from_ref(write), None, now));
                for write in ended {
                    answer(write, Err(Error::Expired));
                }
                writes = keep;
            }
            // The TUN takes a compatible prefix per write; at least one.
            Ok(Some(Ok(written))) => {
                let written = written.clamp(1, writes.len());
                for write in writes.drain(..written) {
                    answer(write, Ok(()));
                }
            }
            Ok(Some(Err(error))) => {
                *failed = failed.saturating_add(1);
                if failed.is_power_of_two() {
                    tracing::warn!(%error, failed = *failed, "Exit TUN write failed; packet dropped, its flow kept");
                }
                let write = writes.remove(0);
                answer(write, Err(Error::Tun(error)));
            }
        }
    }
}

struct Reply {
    incarnation: super::ExitIncarnation,
    packet: Vec<u8>,
    guard: Live,
    authority: super::super::super::protection::ProtectionScope,
}

/// A reverse-NAT packet still requires admission in the unique data-plane flow.
pub struct ExitReply {
    pub incarnation: super::ExitIncarnation,
    pub packet: Vec<u8>,
}

pub struct ExitReplyReader {
    receiver: mpsc::Receiver<Reply>,
    stop: CancellationToken,
}

impl ExitReplyReader {
    pub async fn recv(&mut self) -> Result<ExitReply> {
        loop {
            let reply = tokio::select! { biased;
                _ = self.stop.cancelled() => return Err(Error::Expired),
                reply = self.receiver.recv() => reply.ok_or(Error::Expired)?,
            };
            if let Some(reply) = self.admit(reply) {
                return Ok(reply);
            }
        }
    }

    /// The next reply, then those already queued behind it, at most `max`:
    /// the consumer handles a burst in one turn instead of one per wakeup.
    pub async fn recv_batch(&mut self, max: usize) -> Result<Vec<ExitReply>> {
        let mut batch = vec![self.recv().await?];
        while batch.len() < max
            && let Ok(reply) = self.receiver.try_recv()
        {
            batch.extend(self.admit(reply));
        }
        Ok(batch)
    }

    fn admit(&self, reply: Reply) -> Option<ExitReply> {
        (!self.stop.is_cancelled() && reply.guard.valid() && reply.authority.check().is_ok())
            .then_some(ExitReply {
                incarnation: reply.incarnation,
                packet: reply.packet,
            })
    }
}

#[derive(Clone)]
pub(super) struct Writer {
    sender: mpsc::Sender<Vec<Write>>,
    stop: CancellationToken,
    mtu: usize,
}

impl Writer {
    fn admit(&self, packet: &[u8]) -> Result<()> {
        if self.stop.is_cancelled() {
            return Err(Error::Expired);
        }
        if packet.len() < 20 || packet.len() > self.mtu {
            return Err(Error::Target);
        }
        Ok(())
    }

    fn queue(&self, writes: Vec<Write>) -> Result<()> {
        self.sender.try_send(writes).map_err(|error| match error {
            mpsc::error::TrySendError::Full(_) => Error::Capacity,
            mpsc::error::TrySendError::Closed(_) => Error::Expired,
        })
    }

    /// Write one packet and wait for its own result.
    pub async fn send(
        &self,
        packet: Vec<u8>,
        authority: Authority,
        live: Live,
        packet_guard: Box<dyn super::ExitSendGuard>,
    ) -> Result<()> {
        self.admit(&packet)?;
        let (reply, result) = oneshot::channel();
        self.queue(vec![Write {
            packet,
            reply: Some(reply),
            authority: Some(authority),
            live,
            packet_guard,
        }])?;
        tokio::select! { biased;
            _ = self.stop.cancelled() => Err(Error::Expired),
            result = result => result.map_err(|_| Error::Worker)?,
        }
    }

    /// Queue writes of installed projections, at most `WRITE_BATCH`, as one
    /// batch without waiting for them. `Capacity`: the queue is full and
    /// none was queued.
    pub fn submit(&self, writes: Vec<ExitWrite>) -> Result<()> {
        if writes.len() > WRITE_BATCH {
            return Err(Error::Capacity);
        }
        if writes.is_empty() {
            return Ok(());
        }
        self.queue(writes.into_iter().map(|ExitWrite(write)| write).collect())
    }
}

/// The writer of one live projection, taken just before its packet is
/// released.
pub struct ExitWriter {
    pub(super) live: Live,
    pub(super) writer: Writer,
}

impl ExitWriter {
    /// The packet as a write of this projection, for one batch of
    /// [`super::ExitRole::submit_writes`]; it keeps `packet_guard` up to its write.
    pub fn write(
        self,
        packet: Vec<u8>,
        packet_guard: impl super::ExitSendGuard,
    ) -> Result<ExitWrite> {
        self.writer.admit(&packet)?;
        Ok(ExitWrite(Write {
            packet,
            reply: None,
            authority: None,
            live: self.live,
            packet_guard: Box::new(packet_guard),
        }))
    }
}

pub(super) struct Owner {
    stop: CancellationToken,
    release: CancellationToken,
    task: tokio::task::JoinHandle<()>,
}

impl Owner {
    pub fn start(
        tun: Tun,
        view: View,
        stop: CancellationToken,
        authority: super::owner::Authority,
    ) -> (Self, Writer, ExitReplyReader) {
        let mtu = tun.mtu();
        let (sender, mut writes) = mpsc::channel::<Vec<Write>>(BATCHES);
        let (replies, receiver) = mpsc::channel(64);
        let stopping = stop.clone();
        let release = CancellationToken::new();
        let released = release.clone();
        let task = tokio::spawn(async move {
            let mut buffer = vec![0; mtu];
            let mut failed = 0;
            loop {
                if stopping.is_cancelled() {
                    break;
                }
                tokio::select! {
                    _ = stopping.cancelled() => break,
                    batch = writes.recv() => {
                        let Some(mut batch) = batch else { break; };
                        while batch.len() < WRITE_BATCH && let Ok(more) = writes.try_recv() {
                            batch.extend(more);
                        }
                        // The role's authority, read once for the batch.
                        let current = batch
                            .iter()
                            .find(|write| write.authority.is_none())
                            .and_then(|write| authority.validate(write.live.tuple, &write.live.deadline).ok());
                        write_batch(&tun, batch, current, &stopping, &mut failed).await;
                    },
                    // A reply is read only with room for it: while the consumer
                    // is behind, replies wait in the TUN's own queue (and its
                    // qdisc), never silently dropped here.
                    (permit, result) = async {
                        let permit = replies.reserve().await.ok();
                        (permit, tun.recv(&mut buffer).await)
                    } => {
                        let Ok(size) = result else { break; };
                        let packet = &buffer[..size];
                        let guard = view.read().ok().and_then(|view| view.iter().find(|(_, live)| live.valid() && live.reply(packet)).map(|(id, live)| (*id, live.clone())));
                        if let Some(permit) = permit
                            && let Some((incarnation, guard)) = guard
                            && let Ok(authority) = authority.validate(guard.tuple, &guard.deadline) {
                            permit.send(Reply { incarnation, guard, authority, packet: packet.to_vec() });
                        }
                    },
                }
            }
            // Positive I/O is stopped, but retain the descriptor until the
            // owner has closed qualification/CT and removed its return routes.
            // This avoids treating automatic route deletion as external damage.
            stopping.cancel();
            released.cancelled().await;
            drop(tun);
        });
        (
            Self {
                stop: stop.clone(),
                release,
                task,
            },
            Writer {
                sender,
                stop: stop.clone(),
                mtu,
            },
            ExitReplyReader { receiver, stop },
        )
    }
    pub async fn shutdown(&mut self) -> Result<()> {
        self.stop.cancel();
        self.release.cancel();
        (&mut self.task).await.map_err(|_| Error::Worker)
    }
}

impl Drop for Owner {
    fn drop(&mut self) {
        self.stop.cancel();
        self.release.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::linux::{
        protection::{Mark, Resources, Shared},
        tasks::Tasks,
    };
    use std::time::Duration;
    use tokio::sync::Semaphore;

    #[tokio::test]
    async fn queued_reply_keeps_original_retirement_deadline_and_authority() {
        let tasks = Tasks::new(4);
        let shared = Arc::new(Shared {
            mark: Mark { value: 1, mask: 1 },
            selectors: Vec::new(),
            conflict: CancellationToken::new(),
            network: Default::default(),
            tasks: tasks.clone(),
            sockets: Arc::new(Semaphore::new(1)),
            data_sockets: Arc::new(Semaphore::new(1)),
            attempts: Arc::new(Semaphore::new(1)),
            resolutions: Arc::new(Semaphore::new(1)),
            execution: RwLock::new(None),
            changes: tokio::sync::watch::channel(None).0,
            cleanups: tokio::sync::watch::channel(None).0,
            ending: Default::default(),
            resources: Resources::new(4),
        });
        let guard = Live {
            tuple: OriginalTuple {
                source: "100.96.0.1".parse().unwrap(),
                destination: "192.0.2.1".parse().unwrap(),
                transport: super::super::ExitTransport::Udp {
                    source_port: 1234,
                    destination_port: 53,
                },
            },
            deadline: super::super::tests::cutoff(30_000),
            scope: ExitScope {
                binding: Digest::from_bytes([1; 32]),
                revoked: CancellationToken::new(),
            },
            retired: CancellationToken::new(),
            gate_until: Instant::now() + Duration::from_secs(20),
        };
        let (sender, receiver) = mpsc::channel(4);
        let mut reader = ExitReplyReader {
            receiver,
            stop: CancellationToken::new(),
        };
        sender
            .try_send(Reply {
                incarnation: super::super::ExitIncarnation::new([1; 32]),
                packet: vec![1],
                guard: guard.clone(),
                authority: shared.bootstrap(),
            })
            .unwrap_or_else(|_| panic!("queue capacity"));
        guard.retired.cancel();
        let mut next = guard.clone();
        next.retired = CancellationToken::new();
        sender
            .try_send(Reply {
                incarnation: super::super::ExitIncarnation::new([2; 32]),
                packet: vec![2],
                guard: next.clone(),
                authority: shared.bootstrap(),
            })
            .unwrap_or_else(|_| panic!("queue capacity"));
        assert_eq!(
            reader.recv().await.unwrap().incarnation,
            super::super::ExitIncarnation::new([2; 32])
        );
        let mut expired = next.clone();
        expired.gate_until = Instant::now();
        sender
            .try_send(Reply {
                incarnation: super::super::ExitIncarnation::new([3; 32]),
                packet: vec![3],
                guard: expired,
                authority: shared.bootstrap(),
            })
            .unwrap_or_else(|_| panic!("queue capacity"));
        sender
            .try_send(Reply {
                incarnation: super::super::ExitIncarnation::new([4; 32]),
                packet: vec![4],
                guard: next,
                authority: shared.bootstrap(),
            })
            .unwrap_or_else(|_| panic!("queue capacity"));
        shared.conflict.cancel();
        drop(sender);
        assert!(reader.recv().await.is_err());
        tasks.shutdown().await.unwrap();
    }
}
