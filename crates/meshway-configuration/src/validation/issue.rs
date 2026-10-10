//! Diagnostics of a configuration check: a closed code, its parameters, the
//! JSON pointer of the offending value and an English fallback message.
use serde::Serialize;
use std::collections::BTreeMap;
use utoipa::ToSchema;

meshway_model::string_enum! {
/// Every diagnostic a configuration check can report. Clients localize by
/// code and parameters; `message` is the English rendering of the same.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum IssueCode {
    // Source text and shape.
    /// JSONC syntax error; params `line`, `column` (1-based).
    InvalidJsonc,
    /// Well-formed JSONC whose shape does not match the configuration; params
    /// `line`, `column` of the offending value. Values are never echoed.
    InvalidShape,
    /// A persisted version cannot be restored as written.
    IncompleteVersion,
    // References and names.
    /// A user reference is unknown, inactive or bound to a different membership; params `user`.
    UnknownUser,
    /// A group key lacks `group:` followed by letters, digits, hyphens, underscores or dots; params `name`.
    InvalidGroupName,
    /// A tag key lacks `tag:` followed by letters, digits, hyphens, underscores or dots; params `name`.
    InvalidTagName,
    /// A referenced group is not defined in `groups`; params `name`.
    UnknownGroup,
    /// A referenced tag is not defined in `tagsOwners`; params `name`.
    UnknownTag,
    /// A tag is repeated within one node identity; params `name`.
    DuplicateTag,
    /// A User identity names an account without an active bound membership;
    /// params `user`.
    IdentityMembershipMissing,
    /// A tag identity contains no tags; params are empty.
    EmptyTagIdentity,
    /// A referenced node is unknown or revoked in the current network; params `node`.
    UnknownNode,
    /// A required concrete selector is not a known `node:<id>` reference; params `node`.
    UnknownNodeSelector,
    /// A retained user reference no longer matches its active bound membership; params `user`.
    RetainedMembershipInactive,
    /// A required source, destination or assignment selector set is empty; params are empty.
    EmptySelectorSet,
    /// Bidirectional access includes a destination outside the Mesh node selector space; params are empty.
    BidirectionalRequiresMesh,
    /// A traffic grant has an empty or invalid protocol/port set; params are empty.
    InvalidPortSet,
    /// A domain suffix cannot be normalized as a valid DNS name; params `value`.
    InvalidDomainSuffix,
    /// An exact domain cannot be normalized as a valid DNS name; params `value`.
    InvalidDomain,
    /// An access CIDR is not in canonical network-prefix form; params `value`.
    NoncanonicalCidr,
    // DNS.
    /// The network DNS suffix is invalid; params are empty.
    InvalidDnsDomain,
    /// A hostname and network suffix do not form a non-IP DNS name of at most 253 bytes; params are empty.
    InvalidNodeDnsName,
    /// `overrideLocal` is true but the default resolver list is empty; params are empty.
    DnsOverrideRequiresDefault,
    /// A default or split-DNS resolver key is not defined in `servers`; params `name`.
    UnknownResolver,
    /// A resolver key is empty or its IP address is unspecified or multicast; params are empty.
    InvalidResolver,
    /// A split-DNS route contains no resolver keys; params are empty.
    EmptyResolverSet,
    /// Multiple split-DNS route keys normalize to the same suffix; params `value`.
    DuplicateDnsSuffix,
    /// A split-DNS or search suffix is not a valid DNS name; params `value`.
    InvalidDnsSuffix,
    // Nodes, addresses and transports.
    /// A hostname is not a 1–63 character DNS label with only letters, digits and interior hyphens; params are empty.
    InvalidHostname,
    /// Another node uses the same normalized hostname; params `hostname`.
    DuplicateHostname,
    /// A node's listener map contains an empty key; params are empty.
    EmptyListenerKey,
    /// A listener or dialer declaration is invalid; params `reason` is the
    /// transport declaration code. `edge_transport_unavailable` additionally
    /// carries `a`, `b`, `routingRule`, with the topology transports pointer.
    InvalidTransport,
    /// Listener credentials or TLS certificate, key, name or validity failed
    /// verification.
    InvalidListenerMaterials,
    /// Address pools in the same family overlap or are duplicated; params are empty.
    OverlappingPools,
    /// An address pool is not a valid canonical unicast range; params `family` (`ipv4` or `ipv6`).
    InvalidPool,
    /// A desired node address is outside the usable range of its configured pools; params `address`.
    AddressOutsidePool,
    /// Multiple nodes have the same desired Mesh address; params `address`.
    DuplicateAddress,
    /// Address reassignment names a node absent from the configuration; params `node`.
    UnknownReassignmentTarget,
    /// Allocation cannot find another usable unoccupied address; params `family`.
    NoAddressAvailable,
    // Routing.
    /// A required waypoint is repeated in a declared route; params `node`.
    RepeatedRequiredNode,
    /// An exit action has no candidate nodes; params are empty.
    EmptyExitSet,
    /// An exit candidate is repeated or also appears in the required waypoints; params `node`.
    RepeatedExit,
    /// A referenced exit node lacks `exitEnabled`; params `node`.
    ExitNotEnabled,
    /// A routing destination CIDR is invalid or not canonical; params `value`.
    InvalidDestinationCidr,
    /// An egress rule supplies no exact names, suffixes, CIDRs or rule-set references; params are empty.
    EmptyRouteMatch,
    /// A routing rule references a rule-set key that is not defined; params `key`.
    UnknownRuleSet,
    /// A remote rule-set URL is not HTTPS or includes credentials or a fragment; params are empty.
    InvalidRuleSource,
    /// A rule-set declaration has an empty key; params are empty.
    EmptyRuleSetKey,
    /// An inline rule set contains no CIDRs, suffixes or exact names; params are empty.
    EmptyInlineRuleSet,
    /// Rule-set preparation or bound execution content exceeds its budget; params include the applicable `maxSets`, `maxBytes` or `maxEntries` limits.
    RuleSetBudget,
    /// Remote rule set content could not be prepared; params `reason` is the
    /// resource problem code.
    RuleSetUnavailable,
    /// Remote rule-set preparation exceeds its total deadline; params `seconds`.
    RuleSetTimeout,
    /// A profile name is empty or reserved for `mesh`, `auto` or `global:` entries; params `name`.
    ReservedProfileName,
    /// An assignment references an undefined or unavailable routing profile; params `name`.
    UnknownProfile,
    /// An assignment's default profile is outside the merged allowed set; params `profile`.
    DefaultNotAllowed,
    /// An automatic selection target or fallback is outside a subject's allowed profiles; params `profile`.
    AutoTargetNotAllowed,
    /// An automatic selection target is unknown or recursively names `auto`; params `profile`.
    AutoTargetUnknown,
    /// An earlier assignment with an overlapping target has a different
    /// default or lock; params `other` (its index).
    ConflictingAssignments,
    /// A country list is empty or contains a value other than an uppercase two-letter code; params `value` identifies an invalid entry and is absent for an empty list.
    InvalidCountryCode,
    /// An exit rule sends a destination the node has no access grant for;
    /// params `node`, `destination`.
    ExitOutsideAccess,
    /// A node is assigned a global exit profile without an Internet access grant; params `node`, `profile`.
    GlobalExitRequiresInternet,
    /// A node is assigned a profile whose exit fallback lacks an Internet access grant; params `node`, `profile`.
    ExitFallbackRequiresInternet,
    /// A Node's netmap would exceed the distribution safety bound; params
    /// `node`, `bytes`, `maxBytes`.
    NetmapLimit,
    /// Compiling the network's netmaps exceeds a fixed compiler budget (Node count or carrier candidates per edge); no params.
    NetmapBudget,
    // Warnings.
    /// Every destination of this rule is matched by earlier rules; params
    /// `by` (index of the last rule that completes the cover).
    ShadowedRule,
}
}

impl IssueCode {
    /// English rendering, parameter-free. Clients substitute `params`.
    pub fn message(self) -> &'static str {
        use IssueCode::*;
        match self {
            InvalidJsonc => "invalid JSONC syntax",
            InvalidShape => "value does not match the configuration shape",
            IncompleteVersion => "persisted configuration is incomplete or noncanonical",
            UnknownUser => "unknown, inactive, or rebound user reference",
            InvalidGroupName => "invalid group name",
            InvalidTagName => "invalid tag name",
            UnknownGroup => "unknown group",
            UnknownTag => "unknown tag",
            DuplicateTag => "duplicate identity tag",
            IdentityMembershipMissing => "Node identity requires an active bound membership",
            EmptyTagIdentity => "Tag identity must have at least one tag",
            UnknownNode => "unknown or revoked Node",
            UnknownNodeSelector => "expected a known concrete Node selector",
            RetainedMembershipInactive => "retained user membership is inactive",
            EmptySelectorSet => "selector set must not be empty",
            BidirectionalRequiresMesh => "bidirectional access requires Mesh endpoints",
            InvalidPortSet => "invalid protocol/port set",
            InvalidDomainSuffix => "invalid domain suffix",
            InvalidDomain => "invalid domain name",
            NoncanonicalCidr => "noncanonical CIDR",
            InvalidDnsDomain => "invalid DNS domain",
            InvalidNodeDnsName => {
                "hostname and DNS domain must form a non-IP DNS name of at most 253 bytes"
            }
            DnsOverrideRequiresDefault => "overrideLocal requires default resolvers",
            UnknownResolver => "unknown resolver",
            InvalidResolver => "invalid resolver",
            EmptyResolverSet => "resolver set must not be empty",
            DuplicateDnsSuffix => "duplicate normalized DNS suffix",
            InvalidDnsSuffix => "invalid DNS suffix",
            InvalidHostname => "expected a single DNS label",
            DuplicateHostname => "duplicate hostname",
            EmptyListenerKey => "listener key must not be empty",
            InvalidTransport => "invalid transport declaration",
            InvalidListenerMaterials => {
                "listener credentials or TLS certificate/key/name/validity are invalid"
            }
            OverlappingPools => "overlapping address pools",
            InvalidPool => "expected a canonical unicast pool",
            AddressOutsidePool => "address outside usable pool",
            DuplicateAddress => "duplicate address",
            UnknownReassignmentTarget => "unknown address reassignment target",
            NoAddressAvailable => "no other usable address is available",
            RepeatedRequiredNode => "repeated required Node",
            EmptyExitSet => "exit candidate set must not be empty",
            RepeatedExit => "repeated exit or exit included in via",
            ExitNotEnabled => "exit Node is not enabled as an exit",
            InvalidDestinationCidr => "invalid or noncanonical destination CIDR",
            EmptyRouteMatch => "empty route match",
            UnknownRuleSet => "unknown rule set",
            InvalidRuleSource => "rule source must be HTTPS without credentials or fragment",
            EmptyRuleSetKey => "empty rule set key",
            EmptyInlineRuleSet => "empty inline rule set",
            RuleSetBudget => "execution rule set content exceeds the network budget",
            RuleSetUnavailable => "remote rule set preparation failed",
            RuleSetTimeout => "remote rule set preparation deadline exceeded",
            ReservedProfileName => "reserved or empty profile name",
            UnknownProfile => "unknown routing profile",
            DefaultNotAllowed => "default is not in the merged allowed set",
            AutoTargetNotAllowed => "auto target outside allowed profiles",
            AutoTargetUnknown => "unknown or recursive auto target",
            ConflictingAssignments => "overlapping assignments have conflicting default or locked",
            InvalidCountryCode => "expected uppercase two-letter country codes",
            ExitOutsideAccess => "assigned exit destination is outside access grants",
            GlobalExitRequiresInternet => "global exit requires internet access grant",
            ExitFallbackRequiresInternet => "exit fallback requires internet access grant",
            NetmapLimit => "netmap exceeds the distribution safety bound",
            NetmapBudget => "netmap compilation exceeds a fixed compiler budget",
            ShadowedRule => "every destination of this rule is matched by earlier rules",
        }
    }
}

meshway_model::object! {
/// Structured configuration error, warning or retained unavailable dependency, without sensitive input values in its message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigurationIssue {
    /// JSON pointer into the configuration input; empty for the document.
    pub path: String,
    /// Stable condition identifier that determines the parameter keys and message rendering.
    pub code: IssueCode,
    /// Parameters named by the code's documentation. Always present.
    pub params: BTreeMap<String, String>,
    /// English rendering of `code`; not for programmatic use.
    pub message: String,
}
}

impl ConfigurationIssue {
    pub fn new(path: impl Into<String>, code: IssueCode) -> Self {
        Self {
            path: path.into(),
            code,
            params: BTreeMap::new(),
            message: code.message().to_owned(),
        }
    }
    pub fn with(mut self, key: &'static str, value: impl ToString) -> Self {
        self.params.insert(key.to_owned(), value.to_string());
        self
    }
}
