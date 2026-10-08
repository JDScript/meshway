//! The planned answer for one source and one destination over a configuration:
//! which rules decide, which paths exist, and why nothing would be permitted.
//! Rules are named by JSON Pointer into the evaluated configuration. This is a
//! plan over declared configuration and trusted inventory, not a measurement,
//! and it does not depend on live demands or DNS samples.
use crate::execution::UnavailableReason;
use crate::object::{object, tagged};
use meshway_model::NodeId;
use meshway_serde::required_nullable as nullable;

tagged! {
/// What the request's target resolved to in the evaluated configuration.
pub enum EvaluatedDestination {
    /// A configured node, named directly or through its Mesh address.
    Node { node_id: NodeId },
    /// An address that is not a node's: an unassigned Mesh pool address, a
    /// private/non-public range, or the public Internet.
    Address { address: String, class: AddressClass },
    /// A DNS name; which addresses it resolves to is unknown to the plan.
    Domain { name: String },
    /// A public destination no rule names: only fallbacks apply.
    Internet,
} }
meshway_serde::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum AddressClass { Mesh, Private, Public }
}

object! { pub struct ProfileEvaluation {
    pub name: String,
    /// The assignment whose default applies, or null for the built-in `mesh`.
    #[serde(deserialize_with = "nullable")] pub assignment: Option<String> [required = true],
    pub allowed: Vec<String>,
    pub locked: bool,
    /// Whether the request named this profile (true) or it is the source's
    /// current selection / default (false).
    pub requested: bool,
} }

object! { pub struct AccessGrantEvaluation {
    /// `/access/<index>`.
    pub rule: String,
    pub traffic: Vec<String>,
    /// Matched through `bidirectional` from the destination side.
    pub reverse: bool,
    /// Authorizes only for some resolutions: an address grant for a domain, a
    /// domain grant for an address.
    pub resolution_dependent: bool,
} }
object! { pub struct AccessEvaluation {
    /// Some grant authorizes the destination independently of resolution.
    pub allowed: bool,
    pub grants: Vec<AccessGrantEvaluation>,
} }

meshway_serde::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum RouteKind { Mesh, Exit, Direct, Unmanaged }
}
object! { pub struct RuleSetMatch { pub key: String, pub entry: String } }
object! { pub struct RouteEvaluation {
    pub kind: RouteKind,
    /// The deciding element: an egress rule, a profile `fallback`, or a Mesh
    /// route. Null when implicit: automatic Mesh routing, the built-in `mesh`
    /// profile leaving external traffic unmanaged, or a `global:` profile.
    #[serde(deserialize_with = "nullable")] pub rule: Option<String> [required = true],
    /// The profile whose rules decided, after `auto` resolution; null for a
    /// public rule or a Mesh destination.
    #[serde(deserialize_with = "nullable")] pub profile: Option<String> [required = true],
    /// The rule-set entry through which the rule matched, when it did.
    #[serde(deserialize_with = "nullable")] pub rule_set: Option<RuleSetMatch> [required = true],
    /// Ordered exit candidates of an exit decision; the Mesh destination itself otherwise.
    pub targets: Vec<NodeId>,
    pub via: Vec<NodeId>,
    /// Earlier rules that were not judged because they match the other kind of
    /// destination (by address for a domain, by name for an address).
    pub resolution_dependent: Vec<String>,
} }

object! { pub struct PlannedEdge {
    pub a: NodeId, pub b: NodeId,
    /// `/topology/<index>` of the allowing rule, null for `defaults.topology`.
    #[serde(deserialize_with = "nullable")] pub topology: Option<String> [required = true],
} }
object! { pub struct PlannedPath { pub nodes: Vec<NodeId>, pub edges: Vec<PlannedEdge> } }

meshway_serde::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum TargetProblem {
    /// `enabled: false`.
    Disabled,
    /// Not `exitEnabled`.
    ExitDisabled,
    /// No trusted keys: never enrolled or credentials revoked.
    NotEnrolled,
    /// Every route is blocked by topology, routing or missing transports.
    NoPath,
    RoutingCycle,
    RoutingLimit,
}
}
object! { pub struct TargetEvaluation {
    pub node_id: NodeId,
    #[serde(deserialize_with = "nullable")] pub problem: Option<TargetProblem> [required = true],
    pub paths: u32,
} }

tagged! {
/// What a demand from the source to the destination would get, as compilation decides it.
pub enum EvaluationOutcome {
    /// Paths exist and access allows the traffic; `paths` lists them, primary first.
    Permitted,
    Direct, Unmanaged,
    Unavailable { reason: UnavailableReason },
} }

object! { pub struct Evaluation {
    pub source: NodeId,
    pub destination: EvaluatedDestination,
    pub profile: ProfileEvaluation,
    pub access: AccessEvaluation,
    /// Null when no route decision applies: an unassigned Mesh address, or a
    /// remote rule set whose content is not available yet.
    #[serde(deserialize_with = "nullable")] pub route: Option<RouteEvaluation> [required = true],
    pub targets: Vec<TargetEvaluation>,
    pub paths: Vec<PlannedPath>,
    pub outcome: EvaluationOutcome,
} }
