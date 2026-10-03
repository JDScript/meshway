use crate::{
    object::{object, tagged, unique_map},
    *,
};
use meshway_model::NodeId;
use meshway_transport_spec::{DialerSet, Listener};
use std::collections::BTreeMap;

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
    Ipv4,
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

object! { #[derive(Default)] pub struct NodeSettingsPatch {
    #[serde(default,skip_serializing_if="Input::is_omitted")] pub hostname: Input<String> [value_type=String],
    #[serde(default,skip_serializing_if="Input::is_omitted")] pub remark: Input<String> [value_type=String],
    #[serde(default,skip_serializing_if="Input::is_omitted")] pub enabled: Input<bool> [value_type=bool],
    #[serde(default,skip_serializing_if="Input::is_omitted")] pub exit_enabled: Input<bool> [value_type=bool],
    #[serde(default,skip_serializing_if="Input::is_omitted")] pub dialers: Input<Option<DialerSet>> [value_type=Option<DialerSet>],
} }
tagged! { pub enum ConfigurationOperation {
    ReplaceAccess { access: Vec<AccessRule> },
    ReplaceTopology { topology: Vec<TopologyRule> },
    ReplaceRouting { routing: RoutingConfiguration },
    ReplaceDns { dns: DnsConfiguration },
    ReplaceDefaults { defaults: Defaults },
    ReplaceGroups { #[serde(deserialize_with="unique_map")] groups: BTreeMap<String,Vec<String>> },
    ReplaceTagsOwners { #[serde(rename="tagsOwners",deserialize_with="unique_map")] tags_owners: BTreeMap<String,Vec<String>> },
    ReplaceRuleSets { #[serde(rename="ruleSets",deserialize_with="unique_map")] rule_sets: BTreeMap<String,RuleSet> },
    SetNodeSettings { #[serde(rename="nodeId")] node_id: NodeId, fields: NodeSettingsPatch },
    SetListener { #[serde(rename="nodeId")] node_id: NodeId, key: String, #[serde(deserialize_with="nullable_listener")] listener: Option<Listener> [required=true] },
    SetIdentityTags { #[serde(rename="nodeId")] node_id: NodeId, tags: Vec<String> },
    SetAddressPools { #[serde(rename="addressPools")] address_pools: AddressPoolsInput },
    SetAddresses { #[serde(rename="nodeId")] node_id: NodeId, addresses: NodeAddressesInput },
    DeleteNode { #[serde(rename="nodeId")] node_id: NodeId },
} }

fn nullable_listener<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Listener>, D::Error> {
    serde::Deserialize::deserialize(d)
}

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
            ConfigurationError::Invalid(vec![ConfigurationIssue {
                path: format!("/nodes/{id}"),
                message: "unknown Node".into(),
            }])
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
                input.defaults = defaults.clone()
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
                    n.dialers = v.clone().map(Input::Value).unwrap_or(Input::Omitted);
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
                        n.listeners.insert(key.clone(), l.clone());
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

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum ChangeKind {
    Added,
    Removed,
    Changed,
}
object! { pub struct ConfigurationChange { pub path: String, pub kind: ChangeKind } }

/// Value comparison only; no raw before/after values escape this function.
/// Arrays count as an ordered section and object addition/removal counts once.
pub fn configuration_changes(
    before: &NetworkConfiguration,
    after: &NetworkConfiguration,
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
