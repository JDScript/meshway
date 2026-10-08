use crate::{
    object::{object, tagged, unique_map},
    *,
};
use meshway_model::NodeId;
use meshway_serde::required_nullable as nullable_listener;
use std::collections::BTreeMap;

/// Address family selected for Mesh address allocation or reassignment.
#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    serde::Serialize,
    serde::Deserialize,
    utoipa::ToSchema,
)]
#[serde(rename_all = "camelCase", try_from = "String")]
pub enum AddressFamily {
    /// Select IPv4 pools and assignments.
    Ipv4,
    /// Select IPv6 pools and assignments.
    Ipv6,
}
impl TryFrom<String> for AddressFamily {
    type Error = &'static str;
    fn try_from(v: String) -> Result<Self, Self::Error> {
        match v.as_str() {
            "ipv4" => Ok(Self::Ipv4),
            "ipv6" => Ok(Self::Ipv6),
            _ => Err("unknown address family"),
        }
    }
}

object! { #[derive(Default)]
/// Typed node settings edits; omitted fields retain their current values.
pub struct NodeSettingsPatch {
    /// Replacement network-unique DNS label; omission retains the current hostname.
    #[serde(default,skip_serializing_if="Input::is_omitted")] pub hostname: Input<String> [value_type=String],
    /// Replacement description; omission retains it and an empty string clears it.
    #[serde(default,skip_serializing_if="Input::is_omitted")] pub remark: Input<String> [value_type=String],
    /// Business execution setting; omission retains it and false withdraws execution without revoking synchronization credentials.
    #[serde(default,skip_serializing_if="Input::is_omitted")] pub enabled: Input<bool> [value_type=bool],
    /// Administrative exit eligibility; omission retains the current setting.
    #[serde(default,skip_serializing_if="Input::is_omitted")] pub exit_enabled: Input<bool> [value_type=bool],
    /// Omission retains the override, null restores network inheritance, and an explicit set replaces the protocol selection, with `{}` disabling active dialing.
    #[serde(default,skip_serializing_if="Input::is_omitted")] pub dialers: Input<Option<DialerSetUpdate>> [value_type=Option<DialerSetUpdate>],
} }
tagged! {
/// Typed edit applied in request order, with per-operation authorization and atomic full-target validation under CAS.
pub enum ConfigurationOperation {
    /// Replace all business access grants.
    ReplaceAccess {
        /// Complete replacement grant list; an empty list denies new Mesh business traffic.
        access: Vec<AccessRule>
    },
    /// Replace all adjacency rules.
    ReplaceTopology {
        /// Complete replacement topology list; an empty list uses the network topology default.
        topology: Vec<TopologyRule>
    },
    /// Replace Mesh and egress routing declarations together.
    ReplaceRouting {
        /// Complete replacement routing settings, including profiles and assignments.
        routing: RoutingConfiguration
    },
    /// Replace network DNS settings.
    ReplaceDns {
        /// Complete replacement naming, resolver and split-DNS configuration.
        dns: DnsConfiguration
    },
    /// Update explicitly provided network default fields.
    ReplaceDefaults {
        /// Default-field updates; omitted fields retain their baseline values.
        defaults: DefaultsUpdate
    },
    /// Replace all user group definitions.
    ReplaceGroups {
        /// `group:<name>` keys mapped to direct member emails; an empty map removes all groups.
        #[serde(deserialize_with="unique_map")] groups: BTreeMap<String,Vec<String>>
    },
    /// Replace all tag definitions and assignment owners.
    ReplaceTagsOwners {
        /// `tag:<name>` keys mapped to owner emails or groups; an empty owner list allows only network administrators to assign the tag.
        #[serde(rename="tagsOwners",deserialize_with="unique_map")] tags_owners: BTreeMap<String,Vec<String>>
    },
    /// Replace all destination rule-set declarations.
    ReplaceRuleSets {
        /// Complete named rule-set map; remote contents are prepared and validated before publication.
        #[serde(rename="ruleSets",deserialize_with="unique_map")] rule_sets: BTreeMap<String,RuleSet>
    },
    /// Update allowed settings of an existing node.
    SetNodeSettings {
        /// Existing node in the target network whose settings are edited.
        #[serde(rename="nodeId")] node_id: NodeId,
        /// Explicit field changes, preserving every omitted setting.
        fields: NodeSettingsPatch
    },
    /// Create, replace or remove one fixed listener.
    SetListener {
        /// Existing node that owns the listener.
        #[serde(rename="nodeId")] node_id: NodeId,
        /// Nonempty node-local listener key.
        key: String,
        /// Replacement listener with secret-retention semantics, or explicit null to remove it; this field is required.
        #[serde(deserialize_with="nullable_listener")] listener: Option<ListenerUpdate> [required=true]
    },
    /// Set a node's tag authorization identity subject to assignment and identity-transition permissions.
    SetIdentityTags {
        /// Existing node whose effective identity is changed.
        #[serde(rename="nodeId")] node_id: NodeId,
        /// Nonempty, duplicate-free `tag:<name>` references; clearing the last tag cannot restore a user identity.
        tags: Vec<String>
    },
    /// Update the network's allocatable Mesh pools.
    SetAddressPools {
        /// Per-family pool replacements; omission retains existing pools and `[]` removes a family's pools.
        #[serde(rename="addressPools")] address_pools: AddressPoolsInput
    },
    /// Update one node's desired Mesh addresses.
    SetAddresses {
        /// Existing node whose address assignments are edited.
        #[serde(rename="nodeId")] node_id: NodeId,
        /// Per-family address updates; omission retains an assignment and null explicitly clears it.
        addresses: NodeAddressesInput
    },
    /// Permanently delete a node, revoke its credentials and retire its addresses on commit.
    DeleteNode {
        /// Existing node to remove permanently; historical restore cannot revive it.
        #[serde(rename="nodeId")] node_id: NodeId
    },
} }

/// Apply typed edits to values. Authorization, material preparation, source-text
/// edits and final validation belong to the transaction preparation pipeline.
pub fn apply_operations(
    input: &mut NetworkConfigurationInput,
    ops: &[ConfigurationOperation],
) -> Result<(), ConfigurationError> {
    fn node(
        input: &mut NetworkConfigurationInput,
        id: NodeId,
    ) -> Result<&mut NodeSettingsInput, ConfigurationError> {
        input.nodes.get_mut(&id).ok_or_else(|| {
            ConfigurationError::Invalid(vec![
                ConfigurationIssue::new(format!("/nodes/{id}"), IssueCode::UnknownNode)
                    .with("node", id),
            ])
        })
    }
    for op in ops {
        match op {
            ConfigurationOperation::ReplaceAccess { access } => input.access = access.clone(),
            ConfigurationOperation::ReplaceTopology { topology } => {
                input.topology = topology.clone()
            }
            ConfigurationOperation::ReplaceRouting { routing } => input.routing = routing.clone(),
            ConfigurationOperation::ReplaceDns { dns } => input.dns = dns.clone(),
            ConfigurationOperation::ReplaceDefaults { defaults } => {
                input.defaults = defaults.apply(&input.defaults)
            }
            ConfigurationOperation::ReplaceGroups { groups } => input.groups = groups.clone(),
            ConfigurationOperation::ReplaceTagsOwners { tags_owners } => {
                input.tags_owners = tags_owners.clone()
            }
            ConfigurationOperation::ReplaceRuleSets { rule_sets } => {
                input.rule_sets = rule_sets.clone()
            }
            ConfigurationOperation::SetAddressPools { address_pools } => {
                input.address_pools = address_pools.clone()
            }
            ConfigurationOperation::SetAddresses { node_id, addresses } => {
                let n = node(input, *node_id)?;
                if let Input::Value(v) = addresses.ipv4 {
                    n.addresses.ipv4 = Input::Value(v);
                }
                if let Input::Value(v) = addresses.ipv6 {
                    n.addresses.ipv6 = Input::Value(v);
                }
            }
            ConfigurationOperation::SetNodeSettings { node_id, fields } => {
                let inherited_dialers = input.defaults.dialers.clone();
                let n = node(input, *node_id)?;
                if let Input::Value(v) = &fields.hostname {
                    n.hostname = v.clone();
                }
                if let Input::Value(v) = &fields.remark {
                    n.remark = v.clone();
                }
                if let Input::Value(v) = fields.enabled {
                    n.enabled = v;
                }
                if let Input::Value(v) = fields.exit_enabled {
                    n.exit_enabled = v;
                }
                if let Input::Value(v) = &fields.dialers {
                    n.dialers = match v {
                        None => Input::Omitted,
                        Some(update) => {
                            let previous = match &n.dialers {
                                Input::Omitted => &inherited_dialers,
                                Input::Value(previous) => previous,
                            };
                            Input::Value(update.apply(previous))
                        }
                    };
                }
            }
            ConfigurationOperation::SetListener {
                node_id,
                key,
                listener,
            } => {
                let n = node(input, *node_id)?;
                match listener {
                    Some(l) => {
                        let listener = l.apply(n.listeners.get(key));
                        n.listeners.insert(key.clone(), listener);
                    }
                    None => {
                        n.listeners.remove(key);
                    }
                }
            }
            ConfigurationOperation::SetIdentityTags { node_id, tags } => {
                node(input, *node_id)?.identity = NodeIdentity::Tag { tags: tags.clone() }
            }
            ConfigurationOperation::DeleteNode { node_id } => {
                node(input, *node_id)?;
                input.nodes.remove(node_id);
            }
        }
    }
    Ok(())
}

meshway_serde::string_enum! {
/// Structural difference at a configuration JSON Pointer.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum ChangeKind {
    /// A member or array element exists only in the target.
    Added,
    /// A member or array element exists only in the baseline.
    Removed,
    /// A value exists in both versions but differs.
    Changed,
}
}
object! {
/// Secret-free summary of one final configuration or stable-binding difference.
pub struct ConfigurationChange {
    /// JSON Pointer to the deepest differing value, or the added or removed member as a whole.
    pub path: String,
    /// Whether the target adds, removes or changes the value at this pointer.
    pub kind: ChangeKind
} }

/// Value comparison only; no raw before/after values escape this function.
/// Objects and arrays are compared member by member, so a change is reported
/// at the deepest pointer that differs; an added or removed member counts once.
pub fn configuration_changes(
    before: &NetworkConfiguration,
    after: &NetworkConfiguration,
) -> Result<Vec<ConfigurationChange>, ConfigurationError> {
    value_changes(before, after)
}

/// Compare normalized declaration plans without pretending pending materials
/// are prepared listeners. Returns paths only, never secret values.
pub fn planned_configuration_changes(
    before: &NetworkConfiguration,
    after: &NetworkConfigurationInput,
) -> Result<Vec<ConfigurationChange>, ConfigurationError> {
    value_changes(&NetworkConfigurationInput::from(before), after)
}

fn value_changes<T: serde::Serialize>(
    before: &T,
    after: &T,
) -> Result<Vec<ConfigurationChange>, ConfigurationError> {
    let a = serde_json::to_value(before).map_err(|_| ConfigurationError::Serialization)?;
    let b = serde_json::to_value(after).map_err(|_| ConfigurationError::Serialization)?;
    let mut changes = Vec::new();
    diff("", Some(&a), Some(&b), &mut changes);
    Ok(changes)
}
fn diff(
    path: &str,
    a: Option<&serde_json::Value>,
    b: Option<&serde_json::Value>,
    out: &mut Vec<ConfigurationChange>,
) {
    if a == b {
        return;
    }
    if let (Some(serde_json::Value::Object(a)), Some(serde_json::Value::Object(b))) = (a, b) {
        let keys: std::collections::BTreeSet<_> = a.keys().chain(b.keys()).collect();
        for k in keys {
            diff(
                &format!("{path}/{}", k.replace('~', "~0").replace('/', "~1")),
                a.get(k),
                b.get(k),
                out,
            );
        }
    } else if let (Some(serde_json::Value::Array(a)), Some(serde_json::Value::Array(b))) = (a, b) {
        for index in 0..a.len().max(b.len()) {
            diff(&format!("{path}/{index}"), a.get(index), b.get(index), out);
        }
    } else {
        out.push(ConfigurationChange {
            path: path.into(),
            kind: if a.is_none() {
                ChangeKind::Added
            } else if b.is_none() {
                ChangeKind::Removed
            } else {
                ChangeKind::Changed
            },
        });
    }
}
