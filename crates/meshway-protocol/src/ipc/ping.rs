//! `meshway ping` targets and the decision and echo facts of a target
//! outside the Mesh. The decision is this Node's own first-packet decision on
//! its installed netmap; none of these values grants or extends authority.
use meshway_configuration::{evaluation::RuleSetMatch, execution::UnavailableReason};
use meshway_model::{NodeId, Sequence};
use serde::Serialize;
use utoipa::ToSchema;

use crate::NodeObservationTargetView;

meshway_model::tagged! {
/// What to ping: a name resolved on this Node, or an address.
#[derive(Clone, Debug, PartialEq, Eq)] pub enum PingTarget {
    /// A Mesh hostname or FQDN of the installed netmap; any other name is resolved through this Node's DNS path.
    Name {
        #[serde(deserialize_with = "target_name")]
        name: String [max_length = 253],
    },
    /// A numeric IP address: a Node's Mesh address or an address outside the Mesh.
    Address {
        #[serde(deserialize_with = "target_address")]
        address: String [max_length = 64],
    },
} }

fn target_name<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    let value = <String as serde::Deserialize>::deserialize(d)?;
    if value.is_empty() || value.len() > 253 {
        return Err(serde::de::Error::custom(
            "ping name must have 1 to 253 bytes",
        ));
    }
    Ok(value)
}

fn target_address<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    let value = <String as serde::Deserialize>::deserialize(d)?;
    if value.len() > 64 || value.parse::<std::net::IpAddr>().is_err() {
        return Err(serde::de::Error::custom(
            "ping address must be a numeric IP address",
        ));
    }
    Ok(value)
}

meshway_model::tagged! {
/// What a new flow toward the destination does on this Node now.
#[derive(Clone, Debug, PartialEq, Eq)] pub enum PingAction {
    /// Through the Mesh to the Node owning the destination.
    Mesh,
    /// Through the Mesh to `node`, the first ready exit of the deciding rule.
    Exit { node: NodeId },
    /// Sent from this Node's own network, not through the Mesh.
    Direct,
    /// Not handled by the Mesh: this Node's own routing applies.
    Unmanaged,
    /// The netmap denies this flow.
    Denied { reason: UnavailableReason },
} }

meshway_model::object! {
/// The egress rule that decides the destination.
#[derive(Clone, Debug, PartialEq, Eq)] pub struct PingRule {
    /// JSON Pointer of the rule in the configuration.
    pub pointer: String,
    /// The rule's remark; empty when it has none.
    pub remark: String,
    /// The rule-set entry through which it matched; null otherwise.
    #[serde(deserialize_with = "meshway_model::wire::required_nullable")]
    pub rule_set: Option<RuleSetMatch> [required = true],
} }

meshway_model::tagged! {
/// The DNS path a name takes on this Node, as its DNS owner selects it.
#[derive(Clone, Debug, PartialEq, Eq)] pub enum PingResolver {
    /// Answered from the netmap's MagicDNS names.
    MagicDns,
    /// Sent to the exit's resolver over the Mesh.
    Exit { node: NodeId },
    /// Sent to configured resolvers, by key, in order.
    Resolvers { keys: Vec<String> },
    /// Left to this Node's own system resolution.
    System,
    /// No DNS path exists for the name on this netmap.
    Unavailable,
} }

meshway_model::object! {
/// A trusted DNS association naming the destination.
#[derive(Clone, Debug, PartialEq, Eq)] pub struct PingAssociation {
    /// Names associated with the destination.
    pub names: Vec<String>,
    /// Whole seconds until the earliest of them ends, when the decision was made.
    pub remaining_seconds: u32,
} }

meshway_model::object! {
/// How a name target resolved on this Node.
#[derive(Clone, Debug, PartialEq, Eq)] pub struct PingResolution {
    pub name: String,
    pub resolver: PingResolver,
    /// Every resolved address, in answer order; empty when it did not resolve.
    pub addresses: Vec<String>,
    /// The association naming the destination; null without one.
    #[serde(deserialize_with = "meshway_model::wire::required_nullable")]
    pub association: Option<PingAssociation> [required = true],
} }

meshway_model::object! {
/// This Node's decision for the target, made on its installed netmap.
#[derive(Clone, Debug, PartialEq, Eq)] pub struct PingDecision {
    /// The address decided; null when a name did not resolve.
    #[serde(deserialize_with = "meshway_model::wire::required_nullable")]
    pub destination: Option<String> [required = true],
    /// The Mesh Node owning the destination; null otherwise.
    #[serde(deserialize_with = "meshway_model::wire::required_nullable")]
    pub node: Option<NodeObservationTargetView> [required = true],
    /// How a name outside the Mesh resolved; null for an address or a Mesh name.
    #[serde(deserialize_with = "meshway_model::wire::required_nullable")]
    pub resolution: Option<PingResolution> [required = true],
    /// The routing profile in effect; null for a Mesh destination.
    #[serde(deserialize_with = "meshway_model::wire::required_nullable")]
    pub profile: Option<String> [required = true],
    /// The deciding egress rule; null for a Mesh destination or the profile fallback.
    #[serde(deserialize_with = "meshway_model::wire::required_nullable")]
    pub rule: Option<PingRule> [required = true],
    /// Null when no destination was decided.
    #[serde(deserialize_with = "meshway_model::wire::required_nullable")]
    pub action: Option<PingAction> [required = true],
    /// The complete path a new flow takes now, this Node first: the circuit
    /// it joins, else the path a new circuit takes; empty outside the Mesh.
    pub path: Vec<NodeId>,
} }

meshway_model::string_enum! {
/// Result of one echo request toward an address outside the Mesh.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum PingEchoOutcome {
    /// An echo reply from the destination.
    Reply,
    /// An ICMP destination unreachable or time exceeded quoting the request.
    Unreachable,
    /// Sent, but neither arrived within the probe deadline.
    Timeout,
    /// Not sent: this Node's admission or socket refused the request.
    Failed,
}
}

meshway_model::object! {
/// ICMP type and code of an error answering an echo request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub struct PingIcmpError {
    pub kind: u8,
    pub code: u8,
} }

meshway_model::object! {
/// One echo request toward an address outside the Mesh.
#[derive(Clone, Debug, PartialEq, Eq)] pub struct PingEcho {
    pub sequence: u16,
    pub outcome: PingEchoOutcome,
    /// Source of the reply or ICMP error; null without one.
    #[serde(deserialize_with = "meshway_model::wire::required_nullable")]
    pub from: Option<String> [required = true],
    /// The ICMP error of an `unreachable` outcome; null otherwise.
    #[serde(deserialize_with = "meshway_model::wire::required_nullable")]
    pub error: Option<PingIcmpError> [required = true],
    /// Monotonic microseconds from admission of the request to receipt of its
    /// answer; null without an answer.
    #[serde(deserialize_with = "meshway_model::wire::required_nullable")]
    pub duration_us: Option<Sequence> [required = true],
    /// The request waited for this Node's circuit setup, which the round trip includes.
    pub setup: bool,
    /// The complete Mesh path the answer returned on, this Node first; empty outside the Mesh.
    pub path: Vec<NodeId>,
} }
