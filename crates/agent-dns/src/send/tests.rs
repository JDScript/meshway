use super::*;
use crate::Zone;
use hickory_proto::{
    op::{Message, MessageType, OpCode, Query},
    rr::{Name, RecordType},
};
use std::{
    collections::BTreeMap,
    net::{Ipv4Addr, Ipv6Addr},
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::{Notify, watch};

struct PendingWriter {
    ready: watch::Sender<bool>,
    waiting: Notify,
    first_would_block: AtomicBool,
    partial: bool,
    attempts: Mutex<Vec<Vec<u8>>>,
    sent: Mutex<Vec<Vec<u8>>>,
}

impl PendingWriter {
    fn new(partial: bool) -> Self {
        let (ready, _) = watch::channel(true);
        Self {
            ready,
            waiting: Notify::new(),
            first_would_block: AtomicBool::new(true),
            partial,
            attempts: Mutex::new(vec![]),
            sent: Mutex::new(vec![]),
        }
    }
    fn unblock(&self) {
        self.ready.send_replace(true);
    }
}

impl Writer for PendingWriter {
    async fn writable(&self) -> io::Result<()> {
        let mut ready = self.ready.subscribe();
        loop {
            if *ready.borrow_and_update() {
                return Ok(());
            }
            self.waiting.notify_one();
            ready.changed().await.unwrap();
        }
    }

    fn try_write(&self, bytes: &[u8]) -> io::Result<usize> {
        self.attempts.lock().unwrap().push(bytes.to_vec());
        if self.first_would_block.swap(false, Ordering::SeqCst) {
            self.ready.send_replace(false);
            return Err(io::ErrorKind::WouldBlock.into());
        }
        let length = if self.partial {
            8.min(bytes.len())
        } else {
            bytes.len()
        };
        self.sent.lock().unwrap().push(bytes[..length].to_vec());
        Ok(length)
    }
}

fn zone(suffix: &str) -> Zone {
    let owner = Name::from_ascii(format!("demo-sha.{suffix}.")).unwrap();
    Zone {
        name: Name::from_ascii(format!("{suffix}.")).unwrap(),
        ttl: 300,
        forward: BTreeMap::from([(
            owner,
            (
                Ipv4Addr::new(100, 64, 0, 2),
                "fd00::2".parse::<Ipv6Addr>().unwrap(),
            ),
        )]),
        reverse: BTreeMap::new(),
    }
}

fn prepared(tcp: bool, lease: Duration) -> (Guard, Response) {
    let guard = Guard::default();
    assert!(guard.publish(zone("meshway.test"), Instant::now() + lease, 1));
    let mut query = Message::new(1567, MessageType::Query, OpCode::Query);
    query.add_query(Query::query(
        Name::from_ascii("demo-sha.meshway.test.").unwrap(),
        RecordType::A,
    ));
    let response = guard.prepare(&query.to_vec().unwrap(), tcp).unwrap();
    (guard, response)
}

fn decode(bytes: &[u8], tcp: bool) -> Message {
    let bytes = if tcp {
        assert_eq!(
            usize::from(u16::from_be_bytes([bytes[0], bytes[1]])),
            bytes.len() - 2
        );
        &bytes[2..]
    } else {
        bytes
    };
    Message::from_vec(bytes).unwrap()
}

#[tokio::test]
async fn pending_udp_and_tcp_old_positive_is_fenced_for_every_withdrawal() {
    for tcp in [false, true] {
        for action in 0..7 {
            let (guard, response) = prepared(tcp, Duration::from_secs(60));
            let writer = Arc::new(PendingWriter::new(false));
            let task_writer = writer.clone();
            let task_guard = guard.clone();
            let task = tokio::spawn(async move {
                transmit(task_writer.as_ref(), &task_guard, response, Instant::now).await
            });
            tokio::time::timeout(Duration::from_secs(2), writer.waiting.notified())
                .await
                .unwrap();
            assert_eq!(writer.attempts.lock().unwrap().len(), 1);
            // This initial positive buffer has been serialized, but WouldBlock
            // means it never reached the kernel. No lock is held by the await.
            assert_eq!(
                decode(&writer.attempts.lock().unwrap()[0], tcp)
                    .answers
                    .len(),
                1
            );
            match action {
                0 => guard.withdraw(),
                1 => guard.disable(),
                2 => {
                    assert!(guard.expire(Instant::now() + Duration::from_secs(61)));
                }
                3 => guard.conflict(1),
                4 => guard.shutdown(),
                5 => {
                    assert!(guard.publish(
                        zone("changed.test"),
                        Instant::now() + Duration::from_secs(60),
                        2
                    ));
                }
                6 => guard.observe_revision(2),
                _ => unreachable!(),
            }
            // Invalidation cancels the readiness wait even before unblocking it.
            tokio::time::timeout(Duration::from_secs(2), task)
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            writer.unblock();
            assert!(writer.sent.lock().unwrap().is_empty());
            assert_eq!(writer.attempts.lock().unwrap().len(), 1);
        }
    }
}

#[tokio::test]
async fn pending_disabled_refusal_is_fenced_by_newer_revision_and_expiry() {
    for tcp in [false, true] {
        for newer_revision in [false, true] {
            let (guard, original) = prepared(tcp, Duration::from_secs(10));
            guard.disable();
            let response = guard
                .prepare(&original.query.to_vec().unwrap(), tcp)
                .unwrap();
            assert!(response.deadline.is_some());
            let writer = Arc::new(PendingWriter::new(false));
            let task_writer = writer.clone();
            let task_guard = guard.clone();
            let task = tokio::spawn(async move {
                transmit(task_writer.as_ref(), &task_guard, response, Instant::now).await
            });
            tokio::time::timeout(Duration::from_secs(2), writer.waiting.notified())
                .await
                .unwrap();
            assert_eq!(
                decode(&writer.attempts.lock().unwrap()[0], tcp)
                    .metadata
                    .response_code,
                hickory_proto::op::ResponseCode::Refused
            );
            if newer_revision {
                guard.observe_revision(2);
            } else {
                assert!(guard.expire(Instant::now() + Duration::from_secs(11)));
            }
            tokio::time::timeout(Duration::from_secs(2), task)
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            writer.unblock();
            assert!(writer.sent.lock().unwrap().is_empty());
        }
    }
}

#[tokio::test]
async fn would_block_discards_serialized_ttl_and_reencodes_at_current_clock() {
    for tcp in [false, true] {
        let (guard, response) = prepared(tcp, Duration::from_secs(60));
        let deadline = response.deadline.unwrap();
        let clock = Arc::new(Mutex::new(deadline - Duration::from_secs(60)));
        let writer = Arc::new(PendingWriter::new(false));
        let task_writer = writer.clone();
        let task_clock = clock.clone();
        let task = tokio::spawn(async move {
            transmit(task_writer.as_ref(), &guard, response, || {
                *task_clock.lock().unwrap()
            })
            .await
        });
        writer.waiting.notified().await;
        assert_eq!(
            decode(&writer.attempts.lock().unwrap()[0], tcp).answers[0].ttl,
            60
        );
        // A controlled clock simulates a large scheduling delay, not a flaky
        // sleep. Readiness must regenerate TTL rather than reuse the old bytes.
        *clock.lock().unwrap() = deadline - Duration::from_secs(17);
        writer.unblock();
        task.await.unwrap().unwrap();
        let sent = writer.sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        let reply = decode(&sent[0], tcp);
        assert_eq!(reply.metadata.id, 1567);
        assert_eq!(reply.answers[0].ttl, 17);
    }
}

#[tokio::test(start_paused = true)]
async fn unready_udp_or_tcp_has_bounded_wait_and_independent_lease_fence() {
    for tcp in [false, true] {
        let (guard, response) = prepared(tcp, Duration::from_secs(60));
        let writer = Arc::new(PendingWriter::new(false));
        let task_writer = writer.clone();
        let task = tokio::spawn(async move {
            transmit(task_writer.as_ref(), &guard, response, Instant::now).await
        });
        writer.waiting.notified().await;
        tokio::time::advance(SEND_TIMEOUT).await;
        task.await.unwrap().unwrap();
        writer.unblock();
        assert!(writer.sent.lock().unwrap().is_empty());

        let (guard, response) = prepared(tcp, Duration::from_millis(1500));
        let writer = Arc::new(PendingWriter::new(false));
        let task_writer = writer.clone();
        let task = tokio::spawn(async move {
            transmit(task_writer.as_ref(), &guard, response, Instant::now).await
        });
        writer.waiting.notified().await;
        // No explicit guard.expire/watchdog call: the send's deadline itself
        // cuts off the old positive while readiness is still stalled.
        tokio::time::advance(Duration::from_millis(500)).await;
        task.await.unwrap().unwrap();
        writer.unblock();
        assert!(writer.sent.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn partial_tcp_frame_is_never_completed_or_retried() {
    let (guard, response) = prepared(true, Duration::from_secs(60));
    let writer = PendingWriter::new(true);
    writer.first_would_block.store(false, Ordering::SeqCst);
    let error = transmit(&writer, &guard, response, Instant::now)
        .await
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::WriteZero);
    assert_eq!(writer.attempts.lock().unwrap().len(), 1);
    let sent = writer.sent.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].len(), 8);
    assert!(usize::from(u16::from_be_bytes([sent[0][0], sent[0][1]])) > sent[0].len() - 2);
}

#[tokio::test(start_paused = true)]
async fn fresh_query_after_expiry_before_watchdog_gets_servfail() {
    let (guard, response) = prepared(false, Duration::from_millis(1500));
    tokio::time::advance(Duration::from_secs(2)).await;
    let fresh = guard
        .prepare(&response.query.to_vec().unwrap(), false)
        .unwrap();
    assert!(fresh.deadline.is_none());
    let writer = PendingWriter::new(false);
    writer.first_would_block.store(false, Ordering::SeqCst);
    transmit(&writer, &guard, fresh, Instant::now)
        .await
        .unwrap();
    let sent = writer.sent.lock().unwrap();
    assert_eq!(sent.len(), 1);
    let reply = decode(&sent[0], false);
    assert!(reply.answers.is_empty());
    assert_eq!(
        reply.metadata.response_code,
        hickory_proto::op::ResponseCode::ServFail
    );
}

#[test]
fn seconds_boundary_at_final_write_reencodes_or_cancels_without_stale_ttl() {
    for tcp in [false, true] {
        for expired in [false, true] {
            let (guard, response) = prepared(tcp, Duration::from_secs(10));
            let deadline = response.deadline.unwrap();
            let times = Mutex::new(std::collections::VecDeque::from([
                deadline - Duration::from_secs(2),
                deadline - Duration::from_secs(1),
                deadline - Duration::from_secs(1),
                deadline
                    - if expired {
                        Duration::from_millis(500)
                    } else {
                        Duration::from_secs(1)
                    },
            ]));
            let writer = PendingWriter::new(false);
            writer.first_would_block.store(false, Ordering::SeqCst);
            let result = guard
                .try_send(&response, &writer, &|| {
                    times.lock().unwrap().pop_front().unwrap()
                })
                .unwrap();
            let sent = writer.sent.lock().unwrap();
            if expired {
                assert_eq!(result, None);
                assert!(sent.is_empty());
                assert!(writer.attempts.lock().unwrap().is_empty());
            } else {
                assert_eq!(result, Some(()));
                assert_eq!(sent.len(), 1);
                assert_eq!(decode(&sent[0], tcp).answers[0].ttl, 1);
            }
        }
    }
}
