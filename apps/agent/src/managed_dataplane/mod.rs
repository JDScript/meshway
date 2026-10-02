//! App-side device/route ownership and event projection for node execution.
#[cfg(test)]
pub(crate) mod exit;
mod options;
pub(crate) mod packet;
#[cfg(test)]
pub(crate) mod tests;
#[cfg(all(test, target_os = "linux"))]
mod throughput;
pub(crate) mod tun;
pub(crate) use meshway_agent_dataplane::{PacketPlan, in_prefix};
#[cfg(test)]
use meshway_dataplane::packet_io::Admission;
pub use options::TunOptions;
