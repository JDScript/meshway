//! Diagnostics of a configuration check: a closed code, its parameters, the
//! JSON pointer of the offending value and an English fallback message.
use serde::Serialize;
use std::collections::BTreeMap;
use utoipa::ToSchema;

meshway_serde::string_enum! {
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
    /// params `user`
    UnknownUser,
    /// params `name`
    InvalidGroupName,
    /// params `name`
    InvalidTagName,
    /// params `name`
    UnknownGroup,
    /// params `name`
    UnknownTag,
    /// params `name`
    DuplicateTag,
    /// A User identity names an account without an active bound membership;
    /// params `user`.
    IdentityMembershipMissing,
    EmptyTagIdentity,
    /// params `node`
    UnknownNode,
    /// params `node`
    UnknownNodeSelector,
    RetainedMembershipInactive,
    EmptySelectorSet,
    BidirectionalRequiresMesh,
    InvalidPortSet,
    /// params `value`
    InvalidDomainSuffix,
    /// params `value`
    InvalidDomain,
    /// params `value`
    NoncanonicalCidr,
    // DNS.
    InvalidDnsDomain,
    InvalidNodeDnsName,
    DnsOverrideRequiresDefault,
    /// params `name`
    UnknownResolver,
    InvalidResolver,
    EmptyResolverSet,
    /// params `value`
    DuplicateDnsSuffix,
    /// params `value`
    InvalidDnsSuffix,
    // Nodes, addresses and transports.
    InvalidHostname,
    /// params `hostname`
    DuplicateHostname,
    EmptyListenerKey,
    /// A listener or dialer declaration is invalid; params `reason` is the
    /// transport declaration code.
    InvalidTransport,
    /// Listener credentials or TLS certificate, key, name or validity failed
    /// verification.
    InvalidListenerMaterials,
    OverlappingPools,
    /// params `family` (`ipv4` | `ipv6`)
    InvalidPool,
    /// params `address`
    AddressOutsidePool,
    /// params `address`
    DuplicateAddress,
    /// params `node`
    UnknownReassignmentTarget,
    /// params `family`
    NoAddressAvailable,
    // Routing.
    /// params `node`
    RepeatedRequiredNode,
    EmptyExitSet,
    /// params `node`
    RepeatedExit,
    /// params `node`
    ExitNotEnabled,
    /// params `value`
    InvalidDestinationCidr,
    EmptyRouteMatch,
    /// params `key`
    UnknownRuleSet,
    InvalidRuleSource,
    EmptyRuleSetKey,
    EmptyInlineRuleSet,
    /// Remote rule set content is larger than the network budget.
    RuleSetBudget,
    /// Remote rule set content could not be prepared; params `reason` is the
    /// resource problem code.
    RuleSetUnavailable,
    RuleSetTimeout,
    /// params `name`
    ReservedProfileName,
    /// params `name`
    UnknownProfile,
    /// params `profile`
    DefaultNotAllowed,
    /// params `profile`
    AutoTargetNotAllowed,
    /// params `profile`
    AutoTargetUnknown,
    /// An earlier assignment with an overlapping target has a different
    /// default or lock; params `other` (its index).
    ConflictingAssignments,
    /// params `value`
    InvalidCountryCode,
    /// An exit rule sends a destination the node has no access grant for;
    /// params `node`, `destination`.
    ExitOutsideAccess,
    /// params `node`, `profile`
    GlobalExitRequiresInternet,
    /// params `node`, `profile`
    ExitFallbackRequiresInternet,
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
            ShadowedRule => "every destination of this rule is matched by earlier rules",
        }
    }
}

meshway_serde::object! {
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigurationIssue {
    /// JSON pointer into the configuration input; empty for the document.
    pub path: String,
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
