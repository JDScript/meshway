//! Instance-scoped Node observation subscriptions. Selectors are resolved by
//! Server; these DTOs do not authorize probes or certify a target address.

use crate::{
    InstanceRequest,
    object::{wire_object, wire_tagged},
    observations::{ObservationPath, ObservationTask},
};
use meshway_configuration::{NodeAddresses, execution::ObservationServiceScope};
use meshway_model::{
    InstanceId, NodeId, ObservationSubscriptionId, Version, observations::MetricKind,
};

wire_tagged! {
/// Target selector resolved by the Server within the authenticated Node's network and diagnostic permissions.
#[derive(Clone, Debug, PartialEq, Eq)] pub enum NodeObservationTarget {
    /// Select a Node by configured hostname or Mesh FQDN.
    Name {
        /// Requested hostname or network FQDN, limited to 253 characters before resolution.
        #[serde(deserialize_with = "target_name")]
        name: String [max_length = 253],
    },
    /// Select the Node assigned an exact Mesh IP address.
    MeshIp {
        /// Requested numeric Mesh IP, limited to 64 characters and not an outer transport-family constraint.
        #[serde(deserialize_with = "target_address")]
        address: String [max_length = 64],
    },
} }

fn bounded_selector<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
    maximum: usize,
) -> Result<String, D::Error> {
    let value = <String as serde::Deserialize>::deserialize(deserializer)?;
    if value.chars().count() > maximum {
        return Err(serde::de::Error::custom("observation selector is too long"));
    }
    Ok(value)
}

fn target_name<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    bounded_selector(d, 253)
}

fn target_address<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    bounded_selector(d, 64)
}

wire_object! {
/// Server-resolved configured target returned with an authorized Node observation subscription.
#[derive(Clone, Debug, PartialEq, Eq)] pub struct NodeObservationTargetView {
    /// Registered Node selected by the requested name or Mesh IP.
    pub node_id: NodeId,
    /// Target's canonical configured hostname.
    pub hostname: String,
    /// Target's configured Mesh addresses from the authorized configuration.
    pub addresses: NodeAddresses,
} }

wire_object! {
/// Node request for an independently leased diagnostic subscription sourced from its authenticated runtime instance.
#[derive(Clone, Debug)] pub struct CreateNodeObservation {
    /// Current source runtime instance to which the subscription is bound.
    pub instance: InstanceRequest,
    /// Independent subscription identifier reused with the same intent on creation retries.
    pub subscription_id: ObservationSubscriptionId,
    /// Configured Node name or exact Mesh IP to resolve under current permissions.
    pub target: NodeObservationTarget,
    /// Exact TCP, UDP or ICMP scope checked by Access and complete-path routing.
    pub service: ObservationServiceScope,
    /// Automatic authorized path selection or a fixed complete ordered path.
    pub selection: ObservationPath,
    /// Requested measurement kinds; unsupported kinds are reported explicitly by the task.
    pub metrics: Vec<MetricKind>,
    /// Requested sampling interval in milliseconds, from 1000 through 30000 and subject to execution budgets.
    #[serde(deserialize_with = "observation_interval")]
    pub interval_ms: u32 [minimum = 1000, maximum = 30000],
} }

fn observation_interval<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
    let interval = <u32 as serde::Deserialize>::deserialize(d)?;
    if !(1000..=30000).contains(&interval) {
        return Err(serde::de::Error::custom(
            "observation interval must be between 1000 and 30000 ms",
        ));
    }
    Ok(interval)
}

wire_object! {
/// Explicit renewal of the caller's existing Node observation subscription under current permissions.
#[derive(Clone, Debug)] pub struct RenewNodeObservation {
    /// Current source instance that owns the subscription.
    pub instance: InstanceRequest,
    /// Requested absolute subscription deadline in milliseconds since the Unix epoch, serialized as UTC RFC 3339 and subject to lease bounds.
    #[serde(rename = "expiresAt", with = "meshway_model::timestamp")]
    pub expires_at_ms: u64 [value_type = meshway_model::timestamp::Timestamp],
} }

wire_object! {
/// Instance qualification required when reading a Node-owned observation subscription.
#[derive(Clone, Debug, PartialEq, Eq)] pub struct NodeObservationQuery {
    /// Current source runtime instance owning the subscription.
    pub instance_id: InstanceId,
    /// Server-issued instance generation checked to reject replaced callers.
    pub generation: Version,
} }

wire_object! {
/// Server projection of the caller's diagnostic subscription and its resolved configured target.
#[derive(Clone, Debug)] pub struct NodeObservationResponse {
    /// Shared observation task projected with this subscription's independent deadline and identifier.
    pub task: ObservationTask,
    /// Configured target resolved and authorized by the Server.
    pub target: NodeObservationTargetView,
} }

wire_object! {
/// Server cancellation result indicating whether another subscriber still requires the shared source task.
#[derive(Clone, Debug)] pub struct NodeObservationCancelled {
    /// The source executor's still-shared task after this subscription ends;
    /// null only when no other consumer remains.
    #[serde(deserialize_with = "meshway_model::wire::required_nullable")]
    pub task: Option<ObservationTask> [required = true],
} }
