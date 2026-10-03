use std::{io, sync::Arc};

use meshway_dataplane::Runtime;
use tokio::{sync::watch, time::Instant};

use super::{PacketPlan, tun::Device};

pub(crate) async fn run(
    device: Arc<Device>,
    runtime: Arc<Runtime>,
    plan: PacketPlan,
    deadline: watch::Receiver<Option<Instant>>,
    name: &str,
    events: Option<&tokio::sync::mpsc::Sender<crate::daemon::SessionEvent>>,
) -> io::Result<()> {
    #[cfg(all(test, target_os = "linux"))]
    super::throughput::observe_runtime(&runtime);
    let run = plan.run(device, runtime, deadline, || {
        super::tun::proof(name, &plan, true, "generation_active", events)
    });
    let observe = async {
        let mut previous = plan.flow_statistics();
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            let statistics = plan.flow_statistics();
            if statistics == previous {
                continue;
            }
            previous = statistics;
            #[derive(serde::Serialize)]
            struct Observation<'a> {
                event: &'static str,
                node_id: &'a str,
                revision: u32,
                digest: &'a str,
                observed_at: chrono::DateTime<chrono::Utc>,
                statistics: meshway_dataplane::flow::FlowStatistics,
            }
            if let Some(events) = events
                && let Ok(json) = serde_json::to_string(&Observation {
                    event: "managed_packet_admission",
                    node_id: &plan.node_id,
                    revision: plan.revision,
                    digest: &plan.digest,
                    observed_at: chrono::Utc::now(),
                    statistics,
                })
            {
                let _ = events.try_send(crate::daemon::SessionEvent::Receipt(json));
            }
        }
    };
    tokio::select! { result=run=>result, ()=observe=>unreachable!() }
}
