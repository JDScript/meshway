use meshway_protocol::ipc::{DataPlane, NetworkMode, Phase, PingError, PingEvent, Status};
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub(super) struct Intent {
    revision: meshway_model::Version,
    desired: watch::Receiver<super::intent::Desired>,
}
impl Intent {
    pub fn capture(desired: watch::Receiver<super::intent::Desired>) -> Self {
        let revision = desired.borrow().revision;
        Self { revision, desired }
    }
    pub fn current(&self) -> bool {
        let desired = self.desired.borrow();
        self.desired.has_changed().is_ok()
            && desired.revision == self.revision
            && desired.up
            && !desired.shutdown
    }
    async fn ended(&self) {
        let mut desired = self.desired.clone();
        let _ = desired
            .wait_for(|value| value.revision != self.revision || !value.up || value.shutdown)
            .await;
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) enum Progress {
    Setup,
    Waiting,
}
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
impl Progress {
    pub fn replace(&mut self, next: Self) -> bool {
        if *self == next {
            false
        } else {
            *self = next;
            true
        }
    }
}

pub(super) fn admission(status: &Status) -> Result<(), PingError> {
    if !cfg!(target_os = "linux") || status.network_mode != NetworkMode::Tun {
        return Err(PingError::Unsupported);
    }
    if !can_queue(status) {
        return Err(PingError::NotRunning);
    }
    Ok(())
}

/// A ping runs locally on the installed netmap: no Server contact needed.
fn can_queue(status: &Status) -> bool {
    status.desired_up
        && status.identity.is_some()
        && status.netmap.is_some()
        && matches!(status.data_plane, DataPlane::Ready | DataPlane::Preparing)
        && !matches!(status.phase, Phase::Blocked | Phase::Down | Phase::Stopping)
}

#[cfg(test)]
pub(in crate::runtime) fn test_intent() -> (watch::Sender<super::intent::Desired>, Intent) {
    let (desired, receiver) = watch::channel(super::intent::Desired {
        enrollment_key: None,
        rotation: None,
        revision: meshway_model::Version::MIN,
        up: true,
        shutdown: false,
        retry_enrollment: false,
        reenroll: false,
        logout: None,
    });
    (desired, Intent::capture(receiver))
}

/// One bounded forwarder per admitted local client, joined by runtime::run.
/// Wait progress includes pacing and result backpressure, independently of a
/// held Control RPC. Only Setup uses the preparation timer; the packet owner
/// enforces netmap and probe deadlines, and reports terminal outcomes here.
pub(super) async fn forward(
    client: mpsc::Sender<PingEvent>,
    mut incoming: mpsc::Receiver<PingEvent>,
    cancellation: CancellationToken,
    mut stop: watch::Receiver<bool>,
    mut progress: watch::Receiver<Progress>,
    intent: Intent,
) {
    let _cancel = cancellation.clone().drop_guard();
    let started = tokio::time::Instant::now();
    let mut setup_started = (*progress.borrow_and_update() == Progress::Setup).then_some(started);
    let mut waiting_started = started;
    let mut progress_open = true;
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(1));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        if !intent.current() {
            let _ = client.try_send(PingEvent::Failed {
                code: PingError::Stopped,
            });
            break;
        }
        let mut event = tokio::select! { biased;
            _ = intent.ended() => PingEvent::Failed { code: PingError::Stopped },
            changed = progress.changed(), if progress_open => {
                if changed.is_err() { progress_open = false; }
                else {
                    match *progress.borrow_and_update() {
                        Progress::Setup => { if setup_started.is_none() { setup_started = Some(tokio::time::Instant::now()); } },
                        Progress::Waiting => setup_started = None,
                    }
                }
                continue;
            },
            // A queued terminal event wins over its producer's cancellation.
            event = incoming.recv() => event.unwrap_or(PingEvent::Failed { code: PingError::Stopped }),
            _ = cancellation.cancelled() => break,
            _ = client.closed() => break,
            _ = stop.wait_for(|stop| *stop) => {
                let _ = client.try_send(PingEvent::Failed { code: PingError::Stopped });
                break;
            },
            _ = tick.tick() => {
                if setup_started.is_some_and(|at| at.elapsed() >= meshway_dataplane::diagnostics::SETUP_TIMEOUT) {
                    tracing::info!(stage = "setup_timeout", elapsed_ms = setup_started.map(|at| at.elapsed().as_millis() as u64), "Diagnostic preparation budget expired");
                    PingEvent::Failed { code: PingError::Timeout }
                } else {
                    let Some(elapsed_ms) = u64::try_from(waiting_started.elapsed().as_millis()).ok().and_then(|ms| meshway_model::Sequence::new(ms).ok()) else { break; };
                    PingEvent::Preparing { elapsed_ms }
                }
            },
        };
        if !intent.current() {
            event = PingEvent::Failed {
                code: PingError::Stopped,
            };
        }
        let terminal = matches!(
            event,
            PingEvent::Completed { .. } | PingEvent::Failed { .. }
        );
        if matches!(event, PingEvent::Sample { .. } | PingEvent::Echo { .. }) {
            waiting_started = tokio::time::Instant::now();
        }
        // This is elapsed waiting time, not connection time or packet RTT.
        if let PingEvent::Preparing { elapsed_ms } = &mut event {
            let Some(elapsed) = u64::try_from(waiting_started.elapsed().as_millis())
                .ok()
                .and_then(|ms| meshway_model::Sequence::new(ms).ok())
            else {
                break;
            };
            *elapsed_ms = elapsed;
        }
        if client.try_send(event).is_err() || terminal {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rapid_down_up_invalidates_forwarder_even_if_latest_intent_is_up() {
        let (desired, intent) = test_intent();
        let (client, mut received) = mpsc::channel(8);
        let (events, incoming) = mpsc::channel(8);
        let (_stop, stopped) = watch::channel(false);
        let (_progress, progress) = watch::channel(Progress::Setup);
        let cancellation = CancellationToken::new();
        let task = tokio::spawn(forward(
            client,
            incoming,
            cancellation.clone(),
            stopped,
            progress,
            intent,
        ));
        assert!(matches!(
            received.recv().await,
            Some(PingEvent::Preparing { .. })
        ));
        desired.send_modify(|value| {
            value.up = false;
            value.revision = meshway_model::Version::new(2).unwrap();
        });
        desired.send_modify(|value| {
            value.up = true;
            value.revision = meshway_model::Version::new(3).unwrap();
        });
        events
            .try_send(PingEvent::Completed {
                sent: 1,
                success: 1,
                timeout: 0,
                failed: 0,
            })
            .unwrap();
        task.await.unwrap();
        assert!(matches!(
            received.recv().await,
            Some(PingEvent::Failed {
                code: PingError::Stopped
            })
        ));
        assert!(received.recv().await.is_none());
        assert!(cancellation.is_cancelled());
    }

    #[tokio::test]
    async fn remote_prepare_has_progress_and_queued_terminal_survives_producer_cancel() {
        let (client, mut received) = mpsc::channel(8);
        let (events, incoming) = mpsc::channel(8);
        let cancellation = CancellationToken::new();
        let (_stop, stopped) = watch::channel(false);
        let (_progress, progress) = watch::channel(Progress::Setup);
        let (_desired, intent) = test_intent();
        let task = tokio::spawn(forward(
            client,
            incoming,
            cancellation.clone(),
            stopped,
            progress,
            intent,
        ));
        let first = tokio::time::timeout(std::time::Duration::from_secs(2), received.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(first, PingEvent::Preparing { .. }));
        events
            .try_send(PingEvent::Completed {
                sent: 1,
                success: 1,
                timeout: 0,
                failed: 0,
            })
            .unwrap();
        cancellation.cancel();
        task.await.unwrap();
        assert!(matches!(
            received.recv().await,
            Some(PingEvent::Completed { .. })
        ));
    }

    #[tokio::test]
    async fn slow_or_closed_local_consumer_cancels_and_joins_the_forwarder() {
        let (client, _received) = mpsc::channel(1);
        client
            .try_send(PingEvent::Preparing {
                elapsed_ms: meshway_model::Sequence::MIN,
            })
            .unwrap();
        let (_events, incoming) = mpsc::channel(8);
        let cancellation = CancellationToken::new();
        let (_stop, stopped) = watch::channel(false);
        let (_progress, progress) = watch::channel(Progress::Setup);
        let (_desired, intent) = test_intent();
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            forward(
                client,
                incoming,
                cancellation.clone(),
                stopped,
                progress,
                intent,
            ),
        )
        .await
        .unwrap();
        assert!(cancellation.is_cancelled());
    }

    #[tokio::test]
    async fn shutdown_emits_terminal_and_cancels_local_intent() {
        let (client, mut received) = mpsc::channel(8);
        let (_events, incoming) = mpsc::channel(8);
        let cancellation = CancellationToken::new();
        let (_stop, stopped) = watch::channel(true);
        let (_progress, progress) = watch::channel(Progress::Setup);
        let (_desired, intent) = test_intent();
        forward(
            client,
            incoming,
            cancellation.clone(),
            stopped,
            progress,
            intent,
        )
        .await;
        assert!(cancellation.is_cancelled());
        assert!(matches!(
            received.recv().await,
            Some(PingEvent::Failed {
                code: PingError::Stopped
            })
        ));
    }
}
