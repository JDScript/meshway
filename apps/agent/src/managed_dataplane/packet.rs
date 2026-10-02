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
    plan.run(device, runtime, deadline, || {
        super::tun::proof(name, &plan, true, "generation_active", events)
    })
    .await
}
