//! The planned answer for one source and one destination over a configuration:
//! which rules decide, which paths exist, and why nothing would be permitted.
//! Rules are named by JSON Pointer into the evaluated configuration. This is a
//! plan over declared configuration and trusted inventory, not a measurement,
//! and it does not depend on live demands or DNS samples.
use crate::execution::UnavailableReason;
use crate::object::{object, tagged};
use meshway_model::NodeId;
use meshway_model::wire::required_nullable as nullable;

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
meshway_model::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
/// Which range an address that is not a node's falls into.
pub enum AddressClass {
    /// Inside a Mesh address pool but assigned to no node.
    Mesh,
    /// Outside the public Internet ranges: private, link-local and similar.
    Private,
    Public,
}
}

object! {
/// The egress profile the plan evaluated and how the source came to use it.
pub struct ProfileEvaluation {
    /// Profile name: `mesh`, `auto`, `global:<nodeId>` or a configured profile key.
    pub name: String,
    /// The assignment whose default applies, or null for the built-in `mesh`.
    #[serde(deserialize_with = "nullable")] pub assignment: Option<String> [required = true],
    pub allowed: Vec<String>,
    pub locked: bool,
    /// Whether the request named this profile (true) or it is the source's
    /// current selection / default (false).
    pub requested: bool,
} }

object! {
/// One Access rule that covers the destination for the source.
pub struct AccessGrantEvaluation {
    /// `/access/<index>`.
    pub rule: String,
    /// The rule's `ip` entries: protocol/port grants such as `*`, `tcp:443`, `udp:*`.
    pub traffic: Vec<String>,
    /// Matched through `bidirectional` from the destination side.
    pub reverse: bool,
    /// Authorizes only for some resolutions: an address grant for a domain, a
    /// domain grant for an address.
    pub resolution_dependent: bool,
} }
object! {
/// Whether and by which rules Access allows traffic from the source to the destination.
pub struct AccessEvaluation {
    /// Some grant authorizes the destination independently of resolution.
    pub allowed: bool,
    pub grants: Vec<AccessGrantEvaluation>,
} }

meshway_model::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
/// The action a route decision takes.
pub enum RouteKind {
    /// Routed inside the Mesh to the destination node.
    Mesh,
    /// Forwarded through one of `targets` as the exit.
    Exit,
    /// Sent from the source's own network, bypassing the Mesh.
    Direct,
    /// Not handled by the Mesh at all.
    Unmanaged,
}
}
object! {
/// The rule-set entry through which a route rule matched.
pub struct RuleSetMatch {
    /// Rule set key as referenced by the rule's `ruleSets`.
    pub key: String,
    /// The matching entry: a CIDR, a suffix or an exact name, after normalization.
    pub entry: String,
} }
object! {
/// Where traffic to the destination goes and which configuration element decided.
pub struct RouteEvaluation {
    /// `mesh` for a Mesh destination, otherwise the matched rule's action or the profile fallback.
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
    /// Mandatory waypoints: the exit rule's `via`, or the Mesh route's `via` for a Mesh destination.
    pub via: Vec<NodeId>,
    /// Earlier rules that were not judged because they match the other kind of
    /// destination (by address for a domain, by name for an address).
    pub resolution_dependent: Vec<String>,
} }

object! {
/// One hop of a planned path with the topology rule that allows it.
pub struct PlannedEdge {
    /// The hop's endpoints in path order.
    pub a: NodeId,
    pub b: NodeId,
    /// `/topology/<index>` of the allowing rule, null for `defaults.topology`.
    #[serde(deserialize_with = "nullable")] pub topology: Option<String> [required = true],
} }
object! {
/// A complete path from the source to a target, every hop allowed by topology and reachable by a transport.
pub struct PlannedPath {
    /// Nodes from the source to the target, in order.
    pub nodes: Vec<NodeId>,
    /// Consecutive node pairs of `nodes` with their topology decisions.
    pub edges: Vec<PlannedEdge>,
} }

meshway_model::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
/// Why a target has no planned path.
pub enum TargetProblem {
    /// `enabled: false`.
    Disabled,
    /// Not `exitEnabled`.
    ExitDisabled,
    /// Exit prerequisites are unknown, expired or failed.
    ExitUnavailable,
    /// The requested inner family is unsupported by initial exit execution.
    UnsupportedFamily,
    /// No trusted keys: never enrolled or credentials revoked.
    NotEnrolled,
    /// Every route is blocked by topology, routing or missing transports.
    NoPath,
    RoutingCycle,
    RoutingLimit,
}
}
object! {
/// One exit candidate or Mesh target and whether a path to it exists.
pub struct TargetEvaluation {
    pub node_id: NodeId,
    /// Why no path to this target was planned; null when `paths` is nonzero.
    #[serde(deserialize_with = "nullable")] pub problem: Option<TargetProblem> [required = true],
    /// Number of planned paths to this target, at most four.
    pub paths: u32,
    /// Fresh typed exit failure when reported; absence is unknown, not success.
    pub exit_unavailable_reason: Option<meshway_model::exit::ExitUnavailableReason>,
} }

tagged! {
/// What a demand from the source to the destination would get, as compilation decides it.
pub enum EvaluationOutcome {
    /// Paths exist and access allows the traffic; `paths` lists them, primary first.
    Permitted,
    Direct, Unmanaged,
    Unavailable { reason: UnavailableReason },
} }

object! {
/// The plan for one source and destination; see the path evaluation reference.
pub struct Evaluation {
    pub source: NodeId,
    pub destination: EvaluatedDestination,
    pub profile: ProfileEvaluation,
    pub access: AccessEvaluation,
    /// Null when no route decision applies: an unassigned Mesh address, or a
    /// remote rule set whose content is not available yet.
    #[serde(deserialize_with = "nullable")] pub route: Option<RouteEvaluation> [required = true],
    /// Every target the route names, with its problem or path count.
    pub targets: Vec<TargetEvaluation>,
    /// Planned paths across all targets, the primary path first.
    pub paths: Vec<PlannedPath>,
    pub outcome: EvaluationOutcome,
} }
