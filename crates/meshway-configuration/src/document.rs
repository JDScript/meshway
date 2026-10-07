use crate::*;
use meshway_transport_spec::SecretString;
use std::collections::BTreeMap;

/// Keeps editing text (including comments and secrets) separate from the
/// complete normalized value. Both require protected persistence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigurationDocument {
    pub source_jsonc: SecretString,
    pub configuration: NetworkConfiguration,
}

impl ConfigurationDocument {
    pub fn new(source: String, configuration: NetworkConfiguration) -> Self {
        Self {
            source_jsonc: SecretString::new(source),
            configuration,
        }
    }
    pub fn set_source(&mut self, source: String) {
        self.source_jsonc = SecretString::new(source);
    }
}

/// JSONC is deserialized directly into typed fields; no intermediate JSON map
/// can discard duplicate keys. Parser diagnostics never echo secret input.
pub fn parse_jsonc(source: &str) -> Result<NetworkConfigurationInput, ConfigurationError> {
    jsonc_parser::parse_to_serde_value(source, &Default::default()).map_err(|error| {
        // Syntax errors and shape errors both carry a position. Neither echoes
        // the text: a shape error at a secret field must not leak its value.
        let code = match error.kind() {
            jsonc_parser::errors::ParseErrorKind::Custom(_) => IssueCode::InvalidShape,
            _ => IssueCode::InvalidJsonc,
        };
        ConfigurationError::Invalid(vec![
            ConfigurationIssue::new("", code)
                .with("line", error.line_display())
                .with("column", error.column_display()),
        ])
    })
}
/// Stable field order, sorted map keys, retained ordered routing arrays.
pub fn canonical_json(
    configuration: &NetworkConfiguration,
) -> Result<SecretString, ConfigurationError> {
    serde_json::to_string(configuration)
        .map(SecretString::new)
        .map_err(|_| ConfigurationError::Serialization)
}

/// Restore the exact canonical representation written by `canonical_json`.
/// Refuses incomplete snapshots instead of silently supplying newly introduced
/// defaults. The caller must validate current dependencies with meshway-policy.
pub fn restore_canonical_json(source: &str) -> Result<NetworkConfiguration, ConfigurationError> {
    let value: NetworkConfiguration =
        serde_json::from_str(source).map_err(|_| ConfigurationError::Parse)?;
    if canonical_json(&value)?.expose() != source {
        return Err(ConfigurationError::Invalid(vec![ConfigurationIssue::new(
            "",
            IssueCode::IncompleteVersion,
        )]));
    }
    Ok(value)
}

/// Initial complete network configuration, using only Controller-selected
/// pools and DNS suffix. No listener, Node, exit, material, or ID is invented.
pub fn new_network(
    address_pools: AddressPools,
    dns_domain: String,
) -> Result<NetworkConfiguration, ConfigurationError> {
    let input = NetworkConfigurationInput {
        address_pools: AddressPoolsInput {
            ipv4: Input::Value(address_pools.ipv4),
            ipv6: Input::Value(address_pools.ipv6),
        },
        defaults: Defaults::default(),
        groups: BTreeMap::new(),
        tags_owners: BTreeMap::new(),
        access: vec![AccessRule {
            remark: "默认允许 Mesh 成员互访".into(),
            src: vec!["*".into()],
            dst: vec!["*".into()],
            ip: vec!["*".into()],
            bidirectional: false,
        }],
        nodes: BTreeMap::new(),
        dns: DnsConfiguration {
            magic_dns: true,
            domain: dns_domain,
            servers: BTreeMap::new(),
            default: vec![],
            routes: BTreeMap::new(),
            search_domains: vec![],
            override_local: false,
        },
        topology: vec![],
        routing: RoutingConfiguration::default(),
        rule_sets: BTreeMap::new(),
    };
    // This constructor has no editable policy: only the fixed initial defaults
    // above. Editable configurations require the caller's semantic validator.
    struct InitialPolicy;
    impl ConfigurationSemantics for InitialPolicy {
        fn access_and_topology(
            &self,
            _: &mut NetworkConfiguration,
            _: &ValidationContext<'_>,
            _: &mut ValidationReport,
        ) {
        }
        fn routing(
            &self,
            _: &mut NetworkConfiguration,
            _: &ValidationContext<'_>,
            _: &mut ValidationReport,
        ) {
        }
    }
    normalize_with_semantics(input, &ValidationContext::default(), &InitialPolicy)
        .map(|result| result.configuration)
}
