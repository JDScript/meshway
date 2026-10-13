//! A Node's routing profile selection as its stable inputs decide it: what
//! its assignment allows, what its persistent selection names and which
//! profile is in effect. `meshway-policy` computes it; the Server, the Node
//! and the Console read the same value.
use crate::RouteAction;
use crate::object::object;
use meshway_model::wire::required_nullable as nullable;
use meshway_model::{NodeId, Version};

object! {
/// Routing profile selection of one Node under the current configuration.
pub struct RoutingSelectionState {
    /// Profile the Node's persistent selection names under its current identity; null when it has none.
    #[serde(deserialize_with = "nullable")] pub requested_profile: Option<String> [required = true],
    /// Persistent selection version, the base of the next compare-and-swap; null when there is no selection.
    #[serde(deserialize_with = "nullable")] pub version: Option<Version> [required = true],
    /// Profile in effect: the requested one while it is allowed and not locked out, otherwise the default.
    pub applied_profile: String,
    /// Why the requested profile is not in effect; null when nothing is requested or it applies.
    #[serde(deserialize_with = "nullable")] pub requested_unavailable: Option<SelectionUnavailable> [required = true],
    /// Assignment default, in effect without a usable selection.
    pub default_profile: String,
    /// The assignment pins the default: no other profile can be selected.
    pub locked: bool,
    /// Allowed profiles in assignment order, each with a summary of what it does.
    pub profiles: Vec<ProfileSummary>,
    /// How `auto` resolves while it is the applied profile; null otherwise.
    #[serde(deserialize_with = "nullable")] pub auto: Option<AutoResolution> [required = true],
} }

meshway_model::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
/// Why a selected profile is not the one in effect.
pub enum SelectionUnavailable {
    /// The Node's assignments no longer allow the profile.
    NotAllowed,
    /// The assignment is locked to its default.
    Locked,
}
}

meshway_model::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
/// Built-in or configured kind of a routing profile.
pub enum ProfileKind {
    /// Built-in `mesh`: public rules only, other external traffic unmanaged.
    Mesh,
    /// Built-in `global:<exit-node-id>`: all external traffic through one exit.
    Global,
    /// Built-in `auto`: a region-selected allowed profile.
    Auto,
    /// A profile of `routing.egress.profiles`.
    Custom,
}
}

object! {
/// What one allowed profile does with external traffic, after the public rules.
pub struct ProfileSummary {
    /// Profile name: `mesh`, `auto`, `global:<nodeId>` or a configured profile key.
    pub name: String,
    pub kind: ProfileKind,
    /// The profile's own rules (public rules excluded); zero for built-ins.
    pub rules: u32,
    /// Exit Nodes the profile's rules and fallback name, in first-reference order.
    pub exits: Vec<NodeId>,
    /// Action for traffic no rule matches; null for `auto`, whose resolution decides.
    #[serde(deserialize_with = "nullable")] pub fallback: Option<RouteAction> [required = true],
} }

object! {
/// The concrete profile `auto` applies and why.
pub struct AutoResolution {
    /// Allowed non-`auto` profile in effect.
    pub profile: String,
    pub basis: AutoBasis,
} }

meshway_model::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
/// What decided `auto`'s profile.
pub enum AutoBasis {
    /// No trusted region of the Node is a compilation input yet: the
    /// configured `auto.fallback` applies, whatever its public address is.
    NoTrustedRegion,
}
}
