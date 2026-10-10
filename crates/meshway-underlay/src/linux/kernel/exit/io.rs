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
impl ExitCutoff {
    pub fn valid(&self) -> bool {
        Instant::now() < self.at
            && std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .ok()
                .and_then(|elapsed| u64::try_from(elapsed.as_millis()).ok())
                .is_some_and(|now| now < self.ms)
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
        self.deadline.valid()
            && !self.scope.revoked.is_cancelled()
            && !self.retired.is_cancelled()
            && Instant::now() < self.gate_until
    }
    async fn ended(&self) {
        tokio::select! { biased;
            _ = self.retired.cancelled() => {},
            _ = self.deadline.ended(&self.scope.revoked) => {},
            _ = tokio::time::sleep_until(self.gate_until.into()) => {},
        }
    }
    fn reply(&self, packet: &[u8]) -> bool {
        if packet.len() < 28
            || packet[0] != 0x45
            || usize::from(u16::from_be_bytes([packet[2], packet[3]])) != packet.len()
            || u16::from_be_bytes([packet[6], packet[7]]) & 0x3fff != 0
            || packet[12..16] != self.tuple.destination.octets()
            || packet[16..20] != self.tuple.source.octets()
        {
            return false;
        }
        let (protocol, source, destination) = self.tuple.fields();
        if packet[9] != protocol {
            return false;
        }
        if protocol == 1 {
            packet[20] == 0
                && packet[21] == 0
                && u16::from_be_bytes([packet[24], packet[25]]) == source
                && u16::from_be_bytes([packet[26], packet[27]]) == self.tuple.sequence()
        } else {
            (protocol != 6 || packet.len() >= 40)
                && u16::from_be_bytes([packet[20], packet[21]]) == destination
                && u16::from_be_bytes([packet[22], packet[23]]) == source
        }
    }
}

pub(super) type View = Arc<RwLock<BTreeMap<super::ExitIncarnation, Live>>>;

struct Write {
    packet: Vec<u8>,
    reply: oneshot::Sender<Result<()>>,
    authority: super::super::super::protection::ProtectionScope,
    live: Live,
    packet_guard: Box<dyn super::ExitSendGuard>,
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
            if !self.stop.is_cancelled() && reply.guard.valid() && reply.authority.check().is_ok() {
                return Ok(ExitReply {
                    incarnation: reply.incarnation,
                    packet: reply.packet,
                });
            }
        }
    }
}

#[derive(Clone)]
pub(super) struct Writer {
    sender: mpsc::Sender<Write>,
    stop: CancellationToken,
    mtu: usize,
}

impl Writer {
    pub async fn send(
        &self,
        packet: Vec<u8>,
        authority: super::super::super::protection::ProtectionScope,
        live: Live,
        packet_guard: Box<dyn super::ExitSendGuard>,
    ) -> Result<()> {
        if self.stop.is_cancelled() {
            return Err(Error::Expired);
        }
        if packet.len() < 20 || packet.len() > self.mtu {
            return Err(Error::Target);
        }
        let (reply, result) = oneshot::channel();
        self.sender
            .try_send(Write {
                packet,
                reply,
                authority,
                live,
                packet_guard,
            })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => Error::Capacity,
                mpsc::error::TrySendError::Closed(_) => Error::Expired,
            })?;
        tokio::select! { biased;
            _ = self.stop.cancelled() => Err(Error::Expired),
            result = result => result.map_err(|_| Error::Worker)?,
        }
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
        let (sender, mut writes) = mpsc::channel::<Write>(64);
        let (replies, receiver) = mpsc::channel(64);
        let stopping = stop.clone();
        let release = CancellationToken::new();
        let released = release.clone();
        let task = tokio::spawn(async move {
            let mut buffer = vec![0; mtu];
            loop {
                if stopping.is_cancelled() {
                    break;
                }
                tokio::select! {
                    _ = stopping.cancelled() => break,
                    write = writes.recv() => {
                        let Some(write) = write else { break; };
                        let guard = write.live;
                        let result = match &guard {
                            guard if guard.valid() && write.authority.check().is_ok() && guard.tuple.matches(&write.packet) => {
                                tokio::select! { biased;
                                    _ = stopping.cancelled() => Err(Error::Expired),
                                    _ = guard.ended() => Err(Error::Expired),
                                    _ = write.authority.ended() => Err(Error::Expired),
                                    result = super::guard::checked(write.packet_guard.as_ref(), tun.send(&write.packet)) => result.map_err(|_| Error::Expired).and_then(|result| result.map_err(Error::from)),
                                }
                            }
                            _ => Err(Error::Expired),
                        };
                        let _ = write.reply.send(result);
                    },
                    result = tun.recv(&mut buffer) => {
                        let Ok(size) = result else { break; };
                        let packet = &buffer[..size];
                        let guard = view.read().ok().and_then(|view| view.iter().find(|(_, live)| live.valid() && live.reply(packet)).map(|(id, live)| (*id, live.clone())));
                        if let Some((incarnation, guard)) = guard
                            && let Ok(authority) = authority.validate(guard.tuple, &guard.deadline) {
                            let _ = replies.try_send(Reply { incarnation, guard, authority, packet: packet.to_vec() });
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
            network_changes: tokio::sync::watch::channel(0).0,
            tasks: tasks.clone(),
            sockets: Arc::new(Semaphore::new(1)),
            data_sockets: Arc::new(Semaphore::new(1)),
            attempts: Arc::new(Semaphore::new(1)),
            resolutions: Arc::new(Semaphore::new(1)),
            execution: RwLock::new(None),
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
