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

wire_tagged! { #[derive(Clone, Debug, PartialEq, Eq)] pub enum NodeObservationTarget {
    Name {
        #[serde(deserialize_with = "target_name")]
        name: String [max_length = 253],
    },
    MeshIp {
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

wire_object! { #[derive(Clone, Debug, PartialEq, Eq)] pub struct NodeObservationTargetView {
    pub node_id: NodeId,
    pub hostname: String,
    pub addresses: NodeAddresses,
} }

wire_object! { #[derive(Clone, Debug)] pub struct CreateNodeObservation {
    pub instance: InstanceRequest,
    pub subscription_id: ObservationSubscriptionId,
    pub target: NodeObservationTarget,
    pub service: ObservationServiceScope,
    pub selection: ObservationPath,
    pub metrics: Vec<MetricKind>,
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

wire_object! { #[derive(Clone, Debug)] pub struct RenewNodeObservation {
    pub instance: InstanceRequest,
    #[serde(rename = "expiresAt", with = "meshway_model::timestamp")]
    pub expires_at_ms: u64 [value_type = meshway_model::timestamp::Timestamp],
} }

wire_object! { #[derive(Clone, Debug, PartialEq, Eq)] pub struct NodeObservationQuery {
    pub instance_id: InstanceId,
    pub generation: Version,
} }

wire_object! { #[derive(Clone, Debug)] pub struct NodeObservationResponse {
    pub task: ObservationTask,
    pub target: NodeObservationTargetView,
} }

wire_object! { #[derive(Clone, Debug)] pub struct NodeObservationCancelled {
    /// The source executor's still-shared task after this subscription ends;
    /// null only when no other consumer remains.
    #[serde(deserialize_with = "meshway_serde::required_nullable")]
    pub task: Option<ObservationTask> [required = true],
} }
