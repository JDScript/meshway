use crate::*;
use meshway_transport_spec::SecretString;
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;

/// Keeps editing text (including comments and secrets) separate from the
/// complete normalized value. Both require protected persistence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigurationDocument {
    pub source_jsonc: SecretString,
    pub configuration: NetworkConfiguration,
    pub user_bindings: BTreeMap<String, UserBinding>,
}

impl ConfigurationDocument {
    pub fn new(
        source: String,
        configuration: NetworkConfiguration,
        user_bindings: BTreeMap<String, UserBinding>,
    ) -> Self {
        Self {
            source_jsonc: SecretString::new(source),
            configuration,
            user_bindings,
        }
    }
    pub fn set_source(&mut self, source: String) {
        self.source_jsonc = SecretString::new(source);
    }
}

/// JSONC is deserialized directly into typed fields; no intermediate JSON map
/// can discard duplicate keys. Parser diagnostics never echo secret input.
pub fn parse_jsonc(source: &str) -> Result<NetworkConfigurationInput, ConfigurationError> {
    jsonc_parser::parse_to_serde_value(source, &Default::default())
        .map_err(|_| ConfigurationError::Parse)
}
pub fn parse_document(
    source: String,
    context: &ValidationContext<'_>,
) -> Result<ConfigurationDocument, ConfigurationError> {
    let input = parse_jsonc(&source)?;
    let (configuration, user_bindings) = normalize(input, context)?;
    Ok(ConfigurationDocument {
        source_jsonc: SecretString::new(source),
        configuration,
        user_bindings,
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
/// defaults. Current inventory/revocation checks use `validate_configuration`.
pub fn restore_canonical_json(source: &str) -> Result<NetworkConfiguration, ConfigurationError> {
    let value: NetworkConfiguration =
        serde_json::from_str(source).map_err(|_| ConfigurationError::Parse)?;
    if canonical_json(&value)?.expose() != source {
        return Err(ConfigurationError::Invalid(vec![ConfigurationIssue {
            path: String::new(),
            message: "persisted configuration is incomplete or noncanonical".into(),
        }]));
    }
    Ok(value)
}
pub fn configuration_digest(
    configuration: &NetworkConfiguration,
) -> Result<meshway_model::Digest, ConfigurationError> {
    let json = canonical_json(configuration)?;
    let bytes = Sha256::digest(json.expose().as_bytes());
    let hex = bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    hex.parse().map_err(|_| ConfigurationError::Serialization)
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
    normalize(input, &ValidationContext::default()).map(|(config, _)| config)
}
