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
) -> io::Result<()> {
    meshway_dataplane::packet_io::run_with_activation(
        device,
        runtime,
        plan.admission.clone(),
        deadline,
        || super::tun::proof(name, &plan, true, "generation_active"),
    )
    .await
}
