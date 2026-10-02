//! Bounded side-channel sampler. It never establishes an adjacency or carries
//! packets. Dropping this generation cancels both probes and HTTP reporting.
use super::{Manager, session};
use crate::generated::{self, HttpClient};
use chrono::Utc;
use futures_util::{StreamExt, stream};
use meshway_dataplane::Runtime;
use meshway_protocol::{
    AgentConnectionsSnapshot, LATENCY_BATCH_LIMIT, LATENCY_SAMPLE_INTERVAL_SECONDS,
    LatencyChannelAssociation, LatencyDirection, LatencyOutcome, LatencySample,
    ReportLatencySamplesRequest, local,
};
use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc, Weak,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::{sync::mpsc, task::JoinHandle, time};

const QUEUE_CAPACITY: usize = 256;
const CONCURRENCY: usize = 4;
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);

pub(super) struct Source {
    pub runtime: Weak<Runtime>,
    pub snapshot: AgentConnectionsSnapshot,
    pub epoch: u32,
    pub invalidated: Pin<Box<dyn Future<Output = ()> + Send>>,
}

#[derive(Default, Debug)]
pub(crate) struct Summary {
    pub sampled: u64,
    pub queue_dropped: u64,
    pub report_dropped: u64,
}

#[derive(Default)]
struct Counts {
    sampled: AtomicU64,
    queue_dropped: AtomicU64,
    report_dropped: AtomicU64,
}

impl Manager {
    /// Call after a fresh Applied runtime report has been accepted. The caller
    /// owns this task and restarts it after snapshot/epoch changes. No local CLI
    /// settings are added. Uses the caller's existing protected HTTP client.
    pub(crate) fn start_latency_history(
        &self,
        client: HttpClient,
        epoch: u32,
    ) -> Option<JoinHandle<Summary>> {
        let source = self
            .prober()
            .history_source(self.snapshot.as_ref()?, epoch)?;
        Some(tokio::spawn(run(source, client)))
    }
}

pub(super) async fn run(mut source: Source, client: HttpClient) -> Summary {
    let Ok(generation) = local::credential() else {
        return Summary::default();
    };
    let (tx, rx) = mpsc::channel(QUEUE_CAPACITY);
    let counts = Arc::new(Counts::default());
    let sampling = sample_loop(
        &source.runtime,
        &source.snapshot,
        source.epoch,
        &generation,
        tx,
        &counts,
    );
    let reporting = report_loop(client, rx, &counts);
    tokio::pin!(sampling, reporting);
    tokio::select! {
        biased;
        _ = &mut source.invalidated => {},
        _ = &mut sampling => {},
        _ = &mut reporting => {},
    }
    Summary {
        sampled: counts.sampled.load(Ordering::Relaxed),
        queue_dropped: counts.queue_dropped.load(Ordering::Relaxed),
        report_dropped: counts.report_dropped.load(Ordering::Relaxed),
    }
}

async fn sample_loop(
    runtime: &Weak<Runtime>,
    snapshot: &AgentConnectionsSnapshot,
    epoch: u32,
    generation: &str,
    tx: mpsc::Sender<LatencySample>,
    counts: &Counts,
) {
    let mut interval = time::interval(Duration::from_secs(u64::from(
        LATENCY_SAMPLE_INTERVAL_SECONDS,
    )));
    interval.set_missed_tick_behavior(time::MissedTickBehavior::Skip);
    let mut offset = 0;
    loop {
        interval.tick().await;
        let Some(runtime) = runtime.upgrade() else {
            return;
        };
        let mut peers = runtime.peer_connections();
        peers.sort_by(|a, b| a.peer_node_id.cmp(&b.peer_node_id));
        // Fair rotation also bounds work on very large existing registries.
        let count = peers.len();
        if count > 0 {
            peers.rotate_left(offset % count);
        }
        offset = offset.wrapping_add(LATENCY_BATCH_LIMIT);
        let probes = stream::iter(
            peers
                .into_iter()
                .filter(|p| session::describe(&snapshot.payload, p).is_some())
                .take(LATENCY_BATCH_LIMIT)
                .map(|peer| {
                    let runtime = runtime.clone();
                    async move {
                        measure(
                            &runtime,
                            snapshot,
                            epoch,
                            generation,
                            &peer.peer_node_id,
                            PROBE_TIMEOUT,
                        )
                        .await
                    }
                }),
        )
        .buffer_unordered(CONCURRENCY);
        tokio::pin!(probes);
        while let Some(sample) = probes.next().await {
            if let Some(sample) = sample {
                counts.sampled.fetch_add(1, Ordering::Relaxed);
                match tx.try_send(sample) {
                    Ok(()) => {}
                    Err(mpsc::error::TrySendError::Full(_)) => {
                        counts.queue_dropped.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(mpsc::error::TrySendError::Closed(_)) => return,
                }
            }
        }
    }
}

async fn measure(
    runtime: &Runtime,
    snapshot: &AgentConnectionsSnapshot,
    epoch: u32,
    generation: &str,
    peer: &str,
    timeout: Duration,
) -> Option<LatencySample> {
    // No waiting/dial retry for missing peers. Capacity is a sampling gap.
    // Attempt failures are sanitized local outcomes, never network-loss rates.
    if !runtime
        .peer_connections()
        .iter()
        .any(|p| p.peer_node_id == peer && session::describe(&snapshot.payload, p).is_some())
    {
        return None;
    }
    let measurement_id = local::credential().ok()?;
    let measured_at = Utc::now();
    let result = runtime.probe_neighbor_observed(peer, timeout).await;
    let completed_at = Utc::now();
    let (outcome, channel, rtt_us, failure_reasons) = match result {
        Ok(proof) => {
            let association = session::describe(&snapshot.payload, &proof.connection)?;
            (
                LatencyOutcome::Success,
                vec![LatencyChannelAssociation {
                    transport: association.transport,
                    initiator_node_id: association.initiator_node_id,
                    listener_id: association.listener_id?,
                    candidate_id: association.candidate_id,
                }],
                Some(
                    u32::try_from(proof.round_trip.as_nanos().div_ceil(1000))
                        .ok()?
                        .max(1),
                ),
                vec![],
            )
        }
        Err(meshway_dataplane::Error::Timeout) => (LatencyOutcome::Timeout, vec![], None, vec![]),
        Err(meshway_dataplane::Error::Capacity) => return None,
        Err(error) => {
            use meshway_protocol::LatencyFailureReason as Reason;
            let reason = match error {
                meshway_dataplane::Error::Unavailable | meshway_dataplane::Error::Closed => {
                    Reason::ChannelUnavailable
                }
                meshway_dataplane::Error::Authentication => Reason::AuthenticationFailed,
                _ => Reason::LocalSendFailed,
            };
            (LatencyOutcome::Failed, vec![], None, vec![reason])
        }
    };
    Some(LatencySample {
        measurement_id,
        peer_node_id: peer.into(),
        direction: LatencyDirection::SourceToPeer,
        runtime_epoch: epoch,
        revision: snapshot.payload.revision,
        digest: snapshot.digest.clone(),
        sampler_generation: generation.into(),
        measured_at,
        completed_at,
        outcome,
        failure_reasons,
        channel_associations: channel,
        rtt_us,
    })
}

async fn report_loop(client: HttpClient, mut rx: mpsc::Receiver<LatencySample>, counts: &Counts) {
    while let Some(first) = rx.recv().await {
        let mut samples = vec![first];
        while samples.len() < LATENCY_BATCH_LIMIT {
            let Ok(sample) = rx.try_recv() else {
                break;
            };
            samples.push(sample);
        }
        let count = samples.len() as u64;
        // Typed serde adapter to generated wire structs, not duplicated DTOs.
        let body = match serde_json::to_value(ReportLatencySamplesRequest { samples })
            .ok()
            .and_then(|value| {
                serde_json::from_value::<generated::ReportLatencySamplesRequest>(value).ok()
            }) {
            Some(body) => body,
            None => {
                counts.report_dropped.fetch_add(count, Ordering::Relaxed);
                return;
            }
        };
        let mut saved = false;
        for attempt in 0..3 {
            let response =
                time::timeout(HTTP_TIMEOUT, client.report_latency_samples(body.clone())).await;
            match response {
                Ok(Ok(_)) => {
                    saved = true;
                    break;
                }
                Ok(Err(error))
                    if !match error {
                        generated::ApiOpError::Api(ref e) => e.is_retryable(),
                        generated::ApiOpError::Transport(ref e) => e.is_retryable(),
                    } =>
                {
                    counts.report_dropped.fetch_add(count, Ordering::Relaxed);
                    return;
                }
                _ => time::sleep(Duration::from_secs(1 << attempt)).await,
            }
        }
        if !saved {
            counts.report_dropped.fetch_add(count, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests;
