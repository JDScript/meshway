use crate::{
    fields::*,
    object::{object, tagged, unique_map},
};
use meshway_model::NodeId;
use meshway_model::TransportProtocol;
use meshway_model::wire::required_nullable as nullable;
use meshway_transport_spec::{DialerSet, Listener, Port, PositiveDuration, PreparedListener};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
};
use utoipa::ToSchema;

object! {
/// Canonical, nonoverlapping address pools available for Mesh address allocation.
pub struct AddressPools {
    /// Canonical IPv4 CIDRs; an empty list disables IPv4 allocation.
    pub ipv4: Vec<String>,
    /// Canonical IPv6 CIDRs; an empty list disables IPv6 allocation.
    pub ipv6: Vec<String>
} }
object! { #[derive(Default)]
/// Address pool input that distinguishes retaining a family from clearing its pools.
pub struct AddressPoolsInput {
    /// IPv4 CIDRs; omission retains existing pools or requests initial allocation, while `[]` clears them.
    #[serde(default, skip_serializing_if = "Input::is_omitted")] pub ipv4: Input<Vec<String>> [value_type = Vec<String>],
    /// IPv6 CIDRs; omission retains existing pools or requests initial allocation, while `[]` clears them.
    #[serde(default, skip_serializing_if = "Input::is_omitted")] pub ipv6: Input<Vec<String>> [value_type = Vec<String>],
} }
object! {
/// Desired Mesh addresses, unique within the network and inside its usable pools.
pub struct NodeAddresses {
    /// Desired Mesh IPv4 address; null means this node has no IPv4 assignment.
    #[serde(deserialize_with = "nullable")] pub ipv4: Option<Ipv4Addr> [value_type = Option<String>, required = true, format = "ipv4"],
    /// Desired Mesh IPv6 address; null means this node has no IPv6 assignment.
    #[serde(deserialize_with = "nullable")] pub ipv6: Option<Ipv6Addr> [value_type = Option<String>, required = true, format = "ipv6"],
} }
object! { #[derive(Default)]
/// Per-family address edits with explicit retention, allocation and clearing semantics.
pub struct NodeAddressesInput {
    /// Omission retains an existing IPv4 assignment or allocates initially; null clears it, and an address must be unique and in a usable pool.
    #[serde(default, skip_serializing_if = "Input::is_omitted")] pub ipv4: Input<Option<Ipv4Addr>> [value_type = Option<String>],
    /// Omission retains an existing IPv6 assignment or allocates initially; null clears it, and an address must be unique and in a usable pool.
    #[serde(default, skip_serializing_if = "Input::is_omitted")] pub ipv6: Input<Option<Ipv6Addr>> [value_type = Option<String>],
} }
object! {
/// Network-wide unmatched-rule behavior and inherited outbound transport selection.
pub struct Defaults {
    /// Unmatched Mesh business traffic is denied; defaults to `deny`, the only supported value.
    #[serde(default)] pub access: AccessDefault,
    /// Whether unmatched node pairs may form adjacency; defaults to `allow`.
    #[serde(default)] pub topology: TopologyDefault,
    /// Outbound protocol set inherited by nodes; omission uses the saved platform baseline and `{}` disables active dialing.
    #[serde(default = "DialerSet::network_default")] pub dialers: DialerSet,
} }
impl Default for Defaults {
    fn default() -> Self {
        Self {
            access: AccessDefault::Deny,
            topology: TopologyDefault::Allow,
            dialers: DialerSet::network_default(),
        }
    }
}

tagged! {
/// Exclusive authorization identity used to match a node against policy selectors.
pub enum NodeIdentity {
    /// Authorize the node as a network user and that user's groups.
    User {
        /// Email of an active network member, bound to stable user and membership IDs in the saved version.
        user: String
    },
    /// Authorize the node by tags without inheriting user or group permissions.
    Tag {
        /// Nonempty, duplicate-free `tag:<name>` references defined by `tagsOwners`.
        tags: Vec<String>
    }
} }

object! {
/// An additive business traffic grant; all matching grants are combined.
pub struct AccessRule {
    /// Free-form description, defaulting to an empty string; it is not a rule identifier.
    #[serde(default)] pub remark: String,
    /// Nonempty source selectors: `*`, user email, `group:<name>`, `tag:<name>` or `node:<id>`.
    pub src: Vec<String>,
    /// Nonempty destination selectors: Mesh selectors, canonical CIDRs, `domain:<suffix>` or public-only `autogroup:internet`.
    pub dst: Vec<String>,
    /// Nonempty protocol/port grants such as `*`, `tcp:443`, `tcp:8000-8100`, `udp:*` or `icmp:*`.
    pub ip: Vec<String>,
    /// Allow reverse initiation as well; defaults to false and requires both sides to select only Mesh nodes.
    #[serde(default)] pub bidirectional: bool,
} }
object! {
/// Permission for an undirected adjacency; any matching block precedes the first matching allow.
pub struct TopologyRule {
    /// Free-form description, defaulting to an empty string; duplicate descriptions are allowed.
    #[serde(default)] pub remark: String,
    /// Allow adjacency or block the entire edge regardless of other allow rules.
    pub action: TopologyAction,
    /// One endpoint selector: `*`, user email, `group:<name>`, `tag:<name>` or `node:<id>`.
    pub a: String,
    /// The other endpoint selector, with the same syntax and no directional distinction from `a`.
    pub b: String,
    /// Carriers allowed on this undirected edge, in the order they are tried; omitted means unrestricted in the platform order native_udp, native_tcp, vless, trojan, and `[]` leaves the edge without a carrier.
    #[serde(default, skip_serializing_if = "Input::is_omitted")] pub transports: Input<Vec<TransportProtocol>> [value_type = Vec<TransportProtocol>],
    /// Keep an authenticated adjacency on this allowed edge resident even without traffic; defaults to false (acquired on demand, closed when idle). Ignored on block rules.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")] pub keepalive: bool,
} }
fn yes() -> bool {
    true
}
object! {
/// Canonical settings for a registered node, including prepared listener materials.
pub struct NodeSettings {
    /// Network-unique DNS label of 1–63 lowercase letters, digits or interior hyphens.
    pub hostname: String,
    /// Free-form description, defaulting to an empty string and granting no permissions.
    #[serde(default)] pub remark: String,
    /// Authoritative user or tag identity for policy matching.
    pub identity: NodeIdentity,
    /// Enable business execution; defaults to true, while false still permits authenticated configuration synchronization.
    #[serde(default = "yes")] pub enabled: bool,
    /// Desired Mesh addresses bound to the same configuration version as identity and DNS.
    pub addresses: NodeAddresses,
    /// Fixed inbound resources keyed by a nonempty node-local name; defaults to an empty map.
    #[serde(default, deserialize_with = "unique_map")] pub listeners: BTreeMap<String, PreparedListener>,
    /// Omission inherits network dialers; an explicit set replaces them completely and `{}` disables active dialing.
    #[serde(default, skip_serializing_if = "Input::is_omitted")] pub dialers: Input<DialerSet> [value_type = DialerSet],
    /// Permit selection as an exit node; defaults to false and does not indicate execution readiness.
    #[serde(default)] pub exit_enabled: bool,
} }
object! {
/// Editable settings for an already registered node, before address and material preparation.
pub struct NodeSettingsInput {
    /// Network-unique DNS label of 1–63 letters, digits or interior hyphens, normalized to lowercase.
    pub hostname: String,
    /// Free-form description, defaulting to an empty string and granting no permissions.
    #[serde(default)] pub remark: String,
    /// Requested user or tag identity; identity transitions require current authorization and any required device proof.
    pub identity: NodeIdentity,
    /// Enable business execution; defaults to true, while false retains authenticated configuration synchronization.
    #[serde(default = "yes")] pub enabled: bool,
    /// Per-family desired addresses; omitted families retain existing assignments or request initial allocation.
    #[serde(default)] pub addresses: NodeAddressesInput,
    /// Fixed inbound declarations keyed by a nonempty node-local name; omitted automatic materials are retained or initially generated.
    #[serde(default, deserialize_with = "unique_map")] pub listeners: BTreeMap<String, Listener>,
    /// Omission inherits network dialers; an explicit set replaces them completely and `{}` disables active dialing.
    #[serde(default, skip_serializing_if = "Input::is_omitted")] pub dialers: Input<DialerSet> [value_type = DialerSet],
    /// Permit selection as an exit node; defaults to false and does not indicate execution readiness.
    #[serde(default)] pub exit_enabled: bool,
} }

/// Transport used to send queries to an explicitly configured DNS resolver.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", try_from = "String")]
pub enum DnsTransport {
    /// Send DNS queries in UDP datagrams.
    Udp,
    /// Send DNS queries over TCP.
    Tcp,
}
impl TryFrom<String> for DnsTransport {
    type Error = &'static str;
    fn try_from(v: String) -> Result<Self, Self::Error> {
        match v.as_str() {
            "udp" => Ok(Self::Udp),
            "tcp" => Ok(Self::Tcp),
            _ => Err("unsupported DNS transport"),
        }
    }
}
fn dns_port() -> Port {
    53.try_into().expect("constant port")
}
object! {
/// An explicit upstream resolver identified by its literal IP address.
pub struct DnsServer {
    /// UDP or TCP transport for upstream DNS queries.
    #[serde(rename = "type")] pub transport: DnsTransport,
    /// Literal IPv4 or IPv6 resolver address, avoiding a DNS bootstrap dependency.
    pub server: IpAddr [value_type = String],
    /// Resolver destination port in 1–65535; defaults to 53.
    #[serde(default = "dns_port")] pub server_port: Port,
    /// Retain this resolver's explicit DNS role when an exit is selected; defaults to false.
    #[serde(default)] pub use_with_exit: bool,
} }
object! {
/// Network naming, upstream resolvers and longest-label-suffix DNS routing.
pub struct DnsConfiguration {
    /// Resolve authorized network node names from configured Mesh addresses; defaults to true.
    #[serde(default = "yes", rename = "magicDNS")] pub magic_dns: bool,
    /// Network DNS suffix, normalized as a DNS name and combined with each node's hostname.
    pub domain: String,
    /// Upstream resolvers keyed by the names used in `default` and `routes`; defaults to an empty map.
    #[serde(default, deserialize_with = "unique_map")] pub servers: BTreeMap<String, DnsServer>,
    /// Default resolver keys; defaults to an empty list and must be nonempty when `overrideLocal` is true.
    #[serde(default)] pub default: Vec<String>,
    /// DNS suffixes mapped to nonempty resolver-key lists; the longest matching label-boundary suffix wins.
    #[serde(default, deserialize_with = "unique_map")] pub routes: BTreeMap<String, Vec<String>>,
    /// Additional search suffixes in priority order; defaults to an empty list, with the MagicDNS domain prepended when enabled.
    #[serde(default)] pub search_domains: Vec<String>,
    /// Use default resolvers for ordinary names; defaults to false, preserving system resolution outside MagicDNS and split routes without an exit.
    #[serde(default)] pub override_local: bool,
} }
object! {
/// First-match routing requirement recursively evaluated for each segment's actual endpoints.
pub struct MeshRoute {
    /// Free-form description, defaulting to an empty string; it is not a stable rule identifier.
    #[serde(default)] pub remark: String,
    /// Nonempty source selectors: `*`, user email, `group:<name>`, `tag:<name>` or `node:<id>`.
    pub src: Vec<String>,
    /// Nonempty destination Mesh selectors using the same syntax as `src`.
    pub dst: Vec<String>,
    /// Ordered required `node:<id>` waypoints; segment boundary occurrences are satisfied and actual repeats or cycles are rejected.
    pub via: Vec<String>,
} }
// Fields added after versions were first stored are omitted when empty so a
// stored canonical document still reproduces itself byte for byte.
object! { #[derive(Default)]
/// Union of destination conditions for an egress rule; at least one condition list must be nonempty.
pub struct RouteMatch {
    /// Exact DNS names, excluding subdomains; defaults to an empty list and is omitted from canonical output when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")] pub domain: Vec<String>,
    /// DNS label-boundary suffixes; `example.org` matches itself and `a.example.org`, and the list defaults to empty.
    #[serde(default)] pub domain_suffix: Vec<String>,
    /// Canonical destination IPv4 or IPv6 CIDRs; defaults to an empty list.
    #[serde(default, rename = "ipCIDR")] pub ip_cidr: Vec<String>,
    /// Referenced rule-set keys whose destinations join this match; defaults to an empty list.
    #[serde(default)] pub rule_sets: Vec<String>,
} }
// What a matched egress rule does with the traffic.
tagged! {
/// Action selected by the first matching egress rule.
pub enum RuleAction {
    /// Forward through an authorized exit along an allowed Mesh path.
    Exit {
        /// Nonempty ordered primary and backup `node:<id>` exits with `exitEnabled` set.
        nodes: Vec<String>,
        /// Ordered required `node:<id>` relays before the exit; defaults to an empty list.
        #[serde(default)] via: Vec<String>
    },
    /// Send matching traffic directly through the source node's local network.
    Direct,
} }
// What a profile does with traffic no rule matched; `unmanaged` leaves it to
// the local system and is only meaningful as a fallback.
tagged! {
/// Fallback action for destinations not matched by public or selected-profile rules.
pub enum RouteAction {
    /// Forward through an authorized exit along an allowed Mesh path.
    Exit {
        /// Nonempty ordered primary and backup `node:<id>` exits with `exitEnabled` set.
        nodes: Vec<String>,
        /// Ordered required `node:<id>` relays before the exit; defaults to an empty list.
        #[serde(default)] via: Vec<String>
    },
    /// Send traffic directly through the source node's local network.
    Direct,
    /// Leave unmatched traffic to the local system without Mesh interception.
    Unmanaged,
} }
impl From<RuleAction> for RouteAction {
    fn from(action: RuleAction) -> Self {
        match action {
            RuleAction::Exit { nodes, via } => Self::Exit { nodes, via },
            RuleAction::Direct => Self::Direct,
        }
    }
}
object! {
/// Ordered destination rule evaluated before the selected profile's fallback.
pub struct EgressRule {
    /// Free-form description, defaulting to an empty string; duplicate descriptions are allowed.
    #[serde(default)] pub remark: String,
    /// Destination conditions combined by union, requiring at least one nonempty list.
    #[serde(rename = "match")] pub matcher: RouteMatch,
    /// Action used when this rule is the first match, without falling through on path failure.
    pub action: RuleAction,
} }
object! {
/// Named egress plan evaluated after network-wide public rules.
pub struct RoutingProfile {
    /// First-match destination rules in author order; defaults to an empty list.
    #[serde(default)] pub rules: Vec<EgressRule>,
    /// Action for destinations unmatched by public and profile rules.
    pub fallback: RouteAction,
} }
object! {
/// Allowed egress profiles for matching nodes, merged with overlapping assignments.
pub struct ProfileAssignment {
    /// Nonempty Mesh selectors: `*`, user email, `group:<name>`, `tag:<name>` or `node:<id>`.
    pub target: Vec<String>,
    /// Permitted named profiles or built-ins `mesh`, `auto` and `global:<exit-node-id>`; overlapping assignments take the union.
    pub allowed: Vec<String>,
    /// Default profile, required in the merged allowed set and identical across overlapping assignments.
    pub default: String,
    /// Prevent user profile changes; defaults to false and must agree across overlapping assignments.
    #[serde(default)] pub locked: bool,
} }
object! {
/// Region condition used by the automatic profile selector.
pub struct CountryProfile {
    /// Nonempty list of uppercase two-letter country codes matched against the observed public underlay address's region.
    pub countries: Vec<String>,
    /// Profile selected on a match; it must be allowed and cannot recursively select `auto`.
    pub profile: String
} }
object! {
/// Ordered region-to-profile selection with an explicit fallback.
pub struct AutoProfile {
    /// Country mappings evaluated in first-match order.
    pub rules: Vec<CountryProfile>,
    /// Allowed non-`auto` profile used when no region rule matches or no valid region observation exists.
    pub fallback: String
} }
object! { #[derive(Default)]
/// Public egress rules, named profiles and authorization-constrained profile selection.
pub struct EgressRouting {
    /// First-match public rules evaluated before every selected profile; defaults to an empty list.
    #[serde(default)] pub rules: Vec<EgressRule>,
    /// Administrator-defined profiles keyed by nonreserved names; defaults to an empty map.
    #[serde(default, deserialize_with = "unique_map")] pub profiles: BTreeMap<String, RoutingProfile>,
    /// Node-selector profile assignments; defaults to an empty list.
    #[serde(default)] pub assignments: Vec<ProfileAssignment>,
    /// Region-based profile selector; omission leaves `auto` undefined.
    #[serde(default, skip_serializing_if = "Input::is_omitted")] pub auto: Input<AutoProfile> [value_type = AutoProfile],
} }
object! { #[derive(Default)]
/// Mesh path requirements and external destination routing.
pub struct RoutingConfiguration {
    /// Ordered recursive Mesh path rules; an empty list uses automatic routing over allowed topology.
    #[serde(default)] pub mesh: Vec<MeshRoute>,
    /// External destination rules and profiles; omission supplies empty rules, profiles and assignments.
    #[serde(default)] pub egress: EgressRouting,
} }
/// Remote source format whose supported subset converts exactly to IP and DNS destination sets.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "kebab-case", try_from = "String")]
pub enum RuleSetFormat {
    /// sing-box source versions 1–3 with supported IP CIDR and DNS name rules.
    SingBoxSource,
    /// mihomo domain payload, preserving exact names and supported `+.` suffix entries.
    MihomoDomain,
    /// mihomo IP CIDR payload.
    MihomoIpcidr,
}
impl TryFrom<String> for RuleSetFormat {
    type Error = &'static str;
    fn try_from(v: String) -> Result<Self, Self::Error> {
        match v.as_str() {
            "sing-box-source" => Ok(Self::SingBoxSource),
            "mihomo-domain" => Ok(Self::MihomoDomain),
            "mihomo-ipcidr" => Ok(Self::MihomoIpcidr),
            _ => Err("unsupported rule set format"),
        }
    }
}
fn day() -> PositiveDuration {
    "24h".parse().expect("constant duration")
}
tagged! {
/// Destination set declared inline or downloaded and validated by the Server.
pub enum RuleSet {
    /// Define destinations directly, with at least one nonempty condition list.
    Inline {
        /// Canonical IPv4 or IPv6 destination CIDRs; defaults to an empty list.
        #[serde(default, rename = "ipCIDR")] ip_cidr: Vec<String>,
        /// DNS label-boundary suffixes matching the name itself and its subdomains; defaults to an empty list.
        #[serde(default, rename = "domainSuffix")] domain_suffix: Vec<String>,
        /// Exact DNS names without subdomains; defaults to empty and is omitted from canonical output when empty.
        #[serde(default, skip_serializing_if = "Vec::is_empty")] domain: Vec<String>
    },
    /// Fetch a supported source and bind its immutable normalized content to execution publications.
    Remote {
        /// HTTPS source URL without userinfo or fragment; downloads reject redirects and nonpublic resolved addresses.
        url: String,
        /// Source format to convert without dropping unsupported entries.
        format: RuleSetFormat,
        /// Positive integer duration with `ms`, `s`, `m` or `h`; defaults to `24h`, with scheduling bounded below by one second.
        #[serde(default = "day", rename = "updateInterval")] update_interval: PositiveDuration
    },
} }

object! {
/// Complete canonical network configuration with fixed defaults, addresses and prepared materials.
pub struct NetworkConfiguration {
    /// Network-wide usable Mesh address pools.
    pub address_pools: AddressPools,
    /// Unmatched-rule behavior and inherited dialers; omission supplies platform defaults.
    #[serde(default)] pub defaults: Defaults,
    /// `group:<name>` keys mapped to direct member emails, without nested groups; defaults to an empty map.
    #[serde(default, deserialize_with = "unique_map")] pub groups: BTreeMap<String, Vec<String>>,
    /// `tag:<name>` keys mapped to owner emails or `group:<name>` references; empty owner lists restrict assignment to network administrators.
    #[serde(default, deserialize_with = "unique_map")] pub tags_owners: BTreeMap<String, Vec<String>>,
    /// Additive traffic grants; defaults to an empty list, denying new Mesh business traffic.
    #[serde(default)] pub access: Vec<AccessRule>,
    /// Registered node IDs mapped to canonical settings; adding a key does not enroll a node.
    #[serde(default, deserialize_with = "unique_map")] pub nodes: BTreeMap<NodeId, NodeSettings>,
    /// Network naming and explicit resolver routing.
    pub dns: DnsConfiguration,
    /// Undirected adjacency rules; defaults to an empty list governed by `defaults.topology`.
    #[serde(default)] pub topology: Vec<TopologyRule>,
    /// Mesh path constraints and egress selection; defaults to empty routing declarations.
    #[serde(default)] pub routing: RoutingConfiguration,
    /// Named inline or remote destination sets referenced by routing rules; defaults to an empty map.
    #[serde(default, deserialize_with = "unique_map")] pub rule_sets: BTreeMap<String, RuleSet>,
} }
object! {
/// Complete editable network document before canonicalization and dependency preparation.
pub struct NetworkConfigurationInput {
    /// Address pool edits; omitted families retain existing pools or require initial preparation.
    #[serde(default)] pub address_pools: AddressPoolsInput,
    /// Unmatched-rule behavior and inherited dialers; omission supplies platform defaults.
    #[serde(default)] pub defaults: Defaults,
    /// `group:<name>` keys mapped to direct member emails, without nested groups; defaults to an empty map.
    #[serde(default, deserialize_with = "unique_map")] pub groups: BTreeMap<String, Vec<String>>,
    /// `tag:<name>` keys mapped to owner emails or `group:<name>` references; empty owner lists restrict assignment to network administrators.
    #[serde(default, deserialize_with = "unique_map")] pub tags_owners: BTreeMap<String, Vec<String>>,
    /// Additive traffic grants; defaults to an empty list, denying new Mesh business traffic.
    #[serde(default)] pub access: Vec<AccessRule>,
    /// Registered node IDs mapped to desired settings; removing an existing node permanently deletes it on apply.
    #[serde(default, deserialize_with = "unique_map")] pub nodes: BTreeMap<NodeId, NodeSettingsInput>,
    /// Network naming and explicit resolver routing.
    pub dns: DnsConfiguration,
    /// Undirected adjacency rules; defaults to an empty list governed by `defaults.topology`.
    #[serde(default)] pub topology: Vec<TopologyRule>,
    /// Mesh path constraints and egress selection; defaults to empty routing declarations.
    #[serde(default)] pub routing: RoutingConfiguration,
    /// Named inline or remote destination sets; defaults to an empty map, with referenced remote contents prepared before apply.
    #[serde(default, deserialize_with = "unique_map")] pub rule_sets: BTreeMap<String, RuleSet>,
} }
