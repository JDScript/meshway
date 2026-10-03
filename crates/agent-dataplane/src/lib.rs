//! Node execution plans and generation-scoped packet adaptation. Crypto,
//! framing and forwarding remain in meshway-dataplane.
use meshway_dataplane::{Runtime, packet_io::Device};
use std::{fmt, io, sync::Arc};
use tokio::{sync::watch, time::Instant};
mod plan;
pub use plan::{PacketPlan, in_prefix};

#[derive(Debug)]
pub enum PlanError {
    Invalid(&'static str),
}
impl fmt::Display for PlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(message) => f.write_str(message),
        }
    }
}
impl std::error::Error for PlanError {}

impl PacketPlan {
    /// Activation is an observation hook, not delivery evidence or authorization.
    pub async fn run<D: Device>(
        &self,
        device: Arc<D>,
        runtime: Arc<Runtime>,
        deadline: watch::Receiver<Option<Instant>>,
        activated: impl FnOnce() + Send,
    ) -> io::Result<()> {
        meshway_dataplane::packet_io::run_with_activation(
            device,
            runtime,
            self.fresh(),
            deadline,
            activated,
        )
        .await
    }
}
