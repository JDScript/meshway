use crate::*;
use meshway_model::{MembershipId, NodeId, UserId};
use meshway_transport_spec::{Listener, MaterialRequirement, PreparedListener};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use utoipa::{PartialSchema, ToSchema};

mod addresses;
mod allocation;
mod references;
mod routing;
pub use allocation::allocate_addresses;
pub use references::{domain_suffix_matches, topology_allows};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UserBinding {
    pub user_id: UserId,
    pub membership_id: MembershipId,
}

/// All inventory is scoped and authenticated by the Controller. The library
/// never reads a database or creates an identity from a JSON key.
#[derive(Clone, Default)]
pub struct ValidationContext<'a> {
    pub members: BTreeMap<String, UserBinding>,
    pub nodes: BTreeSet<NodeId>,
    pub previous: Option<&'a NetworkConfiguration>,
    pub expected_user_bindings: BTreeMap<String, UserBinding>,
    /// Stable bindings belonging to `previous`, for name-only identity updates.
    pub previous_user_bindings: BTreeMap<String, UserBinding>,
    pub expected_node_user_bindings: BTreeMap<NodeId, UserBinding>,
    pub previous_node_user_bindings: BTreeMap<NodeId, UserBinding>,
    /// Transaction-authorized registration/identity transition targets.
    pub approved_identities: BTreeMap<NodeId, NodeIdentity>,
    /// Exact normalized TLS declarations verified for certificate/key/name,
    /// validity and irreversible revocation by the Controller.
    pub verified_listeners: BTreeMap<(NodeId, String), Listener>,
    /// Resources explicitly deleted then recreated in this edit batch.
    pub recreated_listeners: BTreeSet<(NodeId, String)>,
    /// Immutable contents selected by the Controller in this transaction.
    pub rule_sets: BTreeMap<String, meshway_model::resources::RuleSetData>,
    pub previous_rule_sets: BTreeMap<String, meshway_model::resources::RuleSetData>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PreparationKind {
    AddressPoolV4,
    AddressPoolV6,
    AddressV4,
    AddressV6,
    IdentityProof,
    Material(MaterialRequirement),
    RuleSetContent,
}
impl utoipa::PartialSchema for PreparationKind {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        use utoipa::openapi::{
            Ref,
            schema::{AdditionalProperties, ObjectBuilder, OneOfBuilder, Type},
        };
        let names = [
            Self::AddressPoolV4,
            Self::AddressPoolV6,
            Self::AddressV4,
            Self::AddressV6,
            Self::IdentityProof,
            Self::RuleSetContent,
        ];
        OneOfBuilder::new()
            .item(
                ObjectBuilder::new()
                    .schema_type(Type::String)
                    .enum_values(Some(names.map(|name| {
                        serde_json::to_value(name).expect("unit enum serializes")
                    }))),
            )
            .item(
                ObjectBuilder::new()
                    .schema_type(Type::Object)
                    .property("material", Ref::from_schema_name("MaterialRequirement"))
                    .required("material")
                    .additional_properties(Some(AdditionalProperties::FreeForm(false))),
            )
            .into()
    }
}
impl ToSchema for PreparationKind {
    fn schemas(
        schemas: &mut Vec<(
            String,
            utoipa::openapi::RefOr<utoipa::openapi::schema::Schema>,
        )>,
    ) {
        schemas.push((
            MaterialRequirement::name().into_owned(),
            MaterialRequirement::schema(),
        ));
        MaterialRequirement::schemas(schemas);
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreparationRequirement {
    pub path: String,
    pub kind: PreparationKind,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfigurationIssue {
    pub path: String,
    pub message: String,
}
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ConfigurationError {
    #[error("invalid JSONC configuration (input redacted)")]
    Parse,
    #[error("configuration serialization failed")]
    Serialization,
    #[error("configuration validation failed: {0:?}")]
    Invalid(Vec<ConfigurationIssue>),
    #[error("configuration preparation required: {0:?}")]
    PreparationRequired(Vec<PreparationRequirement>),
}

#[derive(Default)]
struct Check {
    issues: Vec<ConfigurationIssue>,
    unavailable: Vec<ConfigurationIssue>,
    requirements: Vec<PreparationRequirement>,
    bindings: BTreeMap<String, UserBinding>,
    node_bindings: BTreeMap<NodeId, UserBinding>,
}
impl Check {
    fn dependency(&mut self, path: impl Into<String>, message: impl Into<String>, retained: bool) {
        let issue = ConfigurationIssue {
            path: path.into(),
            message: message.into(),
        };
        if retained {
            self.unavailable.push(issue);
        } else {
            self.issues.push(issue);
        }
    }
    fn issue(&mut self, path: impl Into<String>, message: impl Into<String>) {
        self.issues.push(ConfigurationIssue {
            path: path.into(),
            message: message.into(),
        });
    }
    fn require(&mut self, path: impl Into<String>, kind: PreparationKind) {
        self.requirements.push(PreparationRequirement {
            path: path.into(),
            kind,
        });
    }
    fn finish(&mut self) -> Result<(), ConfigurationError> {
        if !self.issues.is_empty() {
            return Err(ConfigurationError::Invalid(std::mem::take(
                &mut self.issues,
            )));
        }
        if !self.requirements.is_empty() {
            return Err(ConfigurationError::PreparationRequired(std::mem::take(
                &mut self.requirements,
            )));
        }
        Ok(())
    }
}
fn pointer(key: &str) -> String {
    key.replace('~', "~0").replace('/', "~1")
}

/// Normalizes pure fields and validates references. Does not compile paths,
/// authorize a transaction, allocate addresses, generate or verify TLS material.
pub fn normalize(
    input: NetworkConfigurationInput,
    context: &ValidationContext<'_>,
) -> Result<(NetworkConfiguration, BTreeMap<String, UserBinding>), ConfigurationError> {
    let result = normalize_with_dependencies(input, context)?;
    Ok((result.configuration, result.user_bindings))
}

/// Retained but withdrawn dependencies are accepted configuration, never
/// executable permission. Consumers must fail closed for these routes and
/// still check live identity/inventory/publication dependencies when compiling.
pub struct NormalizedConfiguration {
    pub configuration: NetworkConfiguration,
    pub user_bindings: BTreeMap<String, UserBinding>,
    pub node_user_bindings: BTreeMap<NodeId, UserBinding>,
    pub unavailable_dependencies: Vec<ConfigurationIssue>,
}

pub fn normalize_with_dependencies(
    input: NetworkConfigurationInput,
    context: &ValidationContext<'_>,
) -> Result<NormalizedConfiguration, ConfigurationError> {
    normalize_inner(input, context, false).map(|(normalized, _, _)| normalized)
}

/// Checked, non-executable declarations for a read-only preview. Automatic
/// material remains absent: this type cannot be passed to the execution compiler.
pub struct ConfigurationPlan {
    pub input: NetworkConfigurationInput,
    pub view: ConfigurationView,
    pub user_bindings: BTreeMap<String, UserBinding>,
    pub node_user_bindings: BTreeMap<NodeId, UserBinding>,
    pub unavailable_dependencies: Vec<ConfigurationIssue>,
    pub requirements: Vec<PreparationRequirement>,
}

pub fn plan_configuration(
    input: NetworkConfigurationInput,
    context: &ValidationContext<'_>,
) -> Result<ConfigurationPlan, ConfigurationError> {
    let (normalized, pending, requirements) = normalize_inner(input, context, true)?;
    let mut input = NetworkConfigurationInput::from(&normalized.configuration);
    let mut view = ConfigurationView::from(&normalized.configuration);
    for ((node, key), listener) in pending {
        view.nodes
            .get_mut(&node)
            .expect("validated node")
            .listeners
            .insert(key.clone(), (&listener).into());
        input
            .nodes
            .get_mut(&node)
            .expect("validated node")
            .listeners
            .insert(key, listener);
    }
    Ok(ConfigurationPlan {
        input,
        view,
        requirements,
        user_bindings: normalized.user_bindings,
        node_user_bindings: normalized.node_user_bindings,
        unavailable_dependencies: normalized.unavailable_dependencies,
    })
}

type PendingListeners = BTreeMap<(NodeId, String), Listener>;
fn normalize_inner(
    mut input: NetworkConfigurationInput,
    context: &ValidationContext<'_>,
    planning: bool,
) -> Result<
    (
        NormalizedConfiguration,
        PendingListeners,
        Vec<PreparationRequirement>,
    ),
    ConfigurationError,
> {
    let mut check = Check::default();
    let mut pending = BTreeMap::new();
    let pools = addresses::pools(
        input.address_pools,
        context.previous.map(|p| &p.address_pools),
        &mut check,
    );
    if let Err(e) = input.defaults.dialers.validate() {
        check.issue("/defaults/dialers", e.to_string());
    }
    references::definitions(
        &mut input.groups,
        &mut input.tags_owners,
        context,
        &mut check,
    );
    let mut nodes = BTreeMap::new();
    for (id, mut node) in input.nodes {
        let path = format!("/nodes/{id}");
        if !context.nodes.contains(&id) {
            check.issue(&path, "unknown or revoked Node");
        }
        let old = context.previous.and_then(|p| p.nodes.get(&id));
        references::identity(
            id,
            &node.identity,
            &input.tags_owners,
            context,
            &format!("{path}/identity"),
            &mut check,
        );
        let stable_user_rename = match (old.map(|n| &n.identity), &node.identity) {
            (Some(NodeIdentity::User { .. }), NodeIdentity::User { user: after }) => context
                .previous_node_user_bindings
                .get(&id)
                .is_some_and(|binding| context.members.get(after) == Some(binding)),
            _ => false,
        };
        if old.map(|n| &n.identity) != Some(&node.identity)
            && !stable_user_rename
            && context.approved_identities.get(&id) != Some(&node.identity)
        {
            check.require(format!("{path}/identity"), PreparationKind::IdentityProof);
        }
        node.hostname.make_ascii_lowercase();
        if node.hostname.is_empty()
            || node.hostname.len() > 63
            || node.hostname.starts_with('-')
            || node.hostname.ends_with('-')
            || !node
                .hostname
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        {
            check.issue(format!("{path}/hostname"), "expected a single DNS label");
        }
        if let Input::Value(dialers) = &node.dialers
            && let Err(e) = dialers.validate()
        {
            check.issue(format!("{path}/dialers"), e.to_string());
        }
        let addresses = NodeAddresses {
            ipv4: addresses::resolve(
                node.addresses.ipv4,
                old.map(|n| n.addresses.ipv4),
                pools.ipv4.is_empty(),
                &format!("{path}/addresses/ipv4"),
                PreparationKind::AddressV4,
                &mut check,
            ),
            ipv6: addresses::resolve(
                node.addresses.ipv6,
                old.map(|n| n.addresses.ipv6),
                pools.ipv6.is_empty(),
                &format!("{path}/addresses/ipv6"),
                PreparationKind::AddressV6,
                &mut check,
            ),
        };
        let mut listeners = BTreeMap::new();
        for (name, mut listener) in node.listeners {
            let lp = format!("{path}/listeners/{}", pointer(&name));
            if name.is_empty() {
                check.issue(&lp, "listener key must not be empty");
            }
            if !context.recreated_listeners.contains(&(id, name.clone()))
                && let Some(previous) = old.and_then(|n| n.listeners.get(&name))
            {
                inherit_materials(&mut listener, previous.declaration());
            }
            if let Err(e) = listener.normalize() {
                check.issue(&lp, e.to_string());
                continue;
            }
            let verified = context.verified_listeners.get(&(id, name.clone())) == Some(&listener);
            let missing: Vec<_> = listener
                .material_requirements()
                .into_iter()
                .filter(|r| *r != MaterialRequirement::CertificateValidation || !verified)
                .collect();
            if !missing.is_empty() {
                for kind in missing {
                    check.require(&lp, PreparationKind::Material(kind));
                }
                pending.insert((id, name), listener);
                continue;
            }
            match PreparedListener::prepare(listener, verified) {
                Ok(value) => {
                    listeners.insert(name, value);
                }
                Err(e) => check.issue(&lp, e.to_string()),
            }
        }
        nodes.insert(
            id,
            NodeSettings {
                hostname: node.hostname,
                remark: node.remark,
                identity: node.identity,
                enabled: node.enabled,
                addresses,
                listeners,
                dialers: node.dialers,
                exit_enabled: node.exit_enabled,
            },
        );
    }
    let mut config = NetworkConfiguration {
        address_pools: pools,
        defaults: input.defaults,
        groups: input.groups,
        tags_owners: input.tags_owners,
        access: input.access,
        nodes,
        dns: input.dns,
        topology: input.topology,
        routing: input.routing,
        rule_sets: input.rule_sets,
    };
    addresses::nodes(&config, &mut check);
    references::rules(&mut config, context, &mut check);
    routing::validate(&mut config, context, &mut check);
    // Only material completion may be deferred. Identity proof, addresses and
    // immutable rule-set inputs remain mandatory for a meaningful preview.
    let requirements = if planning
        && check
            .requirements
            .iter()
            .all(|r| matches!(r.kind, PreparationKind::Material(_)))
    {
        std::mem::take(&mut check.requirements)
    } else {
        Vec::new()
    };
    check.finish()?;
    Ok((
        NormalizedConfiguration {
            configuration: config,
            user_bindings: check.bindings,
            node_user_bindings: check.node_bindings,
            unavailable_dependencies: check.unavailable,
        },
        pending,
        requirements,
    ))
}

/// Rechecks a persisted complete value against current trusted dependencies.
/// Returns its stable user bindings; this is not an execution compiler.
pub fn validate_configuration(
    configuration: &NetworkConfiguration,
    context: &ValidationContext<'_>,
) -> Result<BTreeMap<String, UserBinding>, ConfigurationError> {
    normalize(NetworkConfigurationInput::from(configuration), context).map(|(_, bindings)| bindings)
}

impl From<&NetworkConfiguration> for NetworkConfigurationInput {
    fn from(c: &NetworkConfiguration) -> Self {
        Self {
            address_pools: AddressPoolsInput {
                ipv4: Input::Value(c.address_pools.ipv4.clone()),
                ipv6: Input::Value(c.address_pools.ipv6.clone()),
            },
            defaults: c.defaults.clone(),
            groups: c.groups.clone(),
            tags_owners: c.tags_owners.clone(),
            access: c.access.clone(),
            nodes: c
                .nodes
                .iter()
                .map(|(id, n)| {
                    (
                        *id,
                        NodeSettingsInput {
                            hostname: n.hostname.clone(),
                            remark: n.remark.clone(),
                            identity: n.identity.clone(),
                            enabled: n.enabled,
                            addresses: NodeAddressesInput {
                                ipv4: Input::Value(n.addresses.ipv4),
                                ipv6: Input::Value(n.addresses.ipv6),
                            },
                            listeners: n
                                .listeners
                                .iter()
                                .map(|(name, l)| (name.clone(), l.declaration().clone()))
                                .collect(),
                            dialers: n.dialers.clone(),
                            exit_enabled: n.exit_enabled,
                        },
                    )
                })
                .collect(),
            dns: c.dns.clone(),
            topology: c.topology.clone(),
            routing: c.routing.clone(),
            rule_sets: c.rule_sets.clone(),
        }
    }
}

/// Preserve omitted material only within an existing listener resource.
/// Callers must not pass a deleted/recreated resource as `previous`.
pub fn inherit_materials(target: &mut Listener, previous: &Listener) {
    fn tls(t: &mut meshway_transport_spec::ListenerTls, p: &meshway_transport_spec::ListenerTls) {
        if t.mode == p.mode {
            if t.certificate.is_none() {
                t.certificate = p.certificate.clone();
            }
            if t.private_key.is_none() {
                t.private_key = p.private_key.clone();
            }
        }
    }
    match (target, previous) {
        (Listener::Vless(t), Listener::Vless(p)) => {
            if t.uuid.is_none() {
                t.uuid = p.uuid.clone();
            }
            tls(&mut t.tls, &p.tls);
        }
        (Listener::Trojan(t), Listener::Trojan(p)) => {
            if t.password.is_none() {
                t.password = p.password.clone();
            }
            tls(&mut t.tls, &p.tls);
        }
        _ => {}
    }
}
