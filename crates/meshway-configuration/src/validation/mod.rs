use crate::*;
use meshway_model::{MembershipId, NodeId, UserId};
use meshway_transport_spec::{Listener, MaterialRequirement, PreparedListener};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use utoipa::{PartialSchema, ToSchema};

mod addresses;
mod allocation;
mod issue;
mod references;
pub use allocation::allocate_addresses;
pub use issue::{ConfigurationIssue, IssueCode};
pub use references::bind_user_reference;

/// Canonical Node hostname accepted by configuration and signed enrollment.
/// Callers which normalize user input do so before signing or validation.
pub fn valid_node_hostname(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 63
        && !name.starts_with('-')
        && !name.ends_with('-')
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

meshway_serde::object! {
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserBinding {
    pub user_id: UserId,
    pub membership_id: MembershipId,
}
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
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
impl<'de> Deserialize<'de> for PreparationKind {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = PreparationKind;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a preparation kind string or a material requirement object")
            }
            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
                match value {
                    "addressPoolV4" => Ok(PreparationKind::AddressPoolV4),
                    "addressPoolV6" => Ok(PreparationKind::AddressPoolV6),
                    "addressV4" => Ok(PreparationKind::AddressV4),
                    "addressV6" => Ok(PreparationKind::AddressV6),
                    "identityProof" => Ok(PreparationKind::IdentityProof),
                    "ruleSetContent" => Ok(PreparationKind::RuleSetContent),
                    _ => Err(E::custom("unknown preparation kind")),
                }
            }
            fn visit_map<M: serde::de::MapAccess<'de>>(
                self,
                map: M,
            ) -> Result<Self::Value, M::Error> {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Material {
                    material: MaterialRequirement,
                }
                Material::deserialize(serde::de::value::MapAccessDeserializer::new(map))
                    .map(|value| PreparationKind::Material(value.material))
            }
        }
        d.deserialize_any(Visitor)
    }
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
meshway_serde::object! {
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparationRequirement {
    pub path: String,
    pub kind: PreparationKind,
}
}
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ConfigurationError {
    /// Persisted canonical JSON that no longer restores; never user input.
    #[error("persisted configuration is incomplete or noncanonical")]
    Parse,
    #[error("configuration serialization failed")]
    Serialization,
    #[error("configuration validation failed: {0:?}")]
    Invalid(Vec<ConfigurationIssue>),
    #[error("configuration preparation required: {0:?}")]
    PreparationRequired(Vec<PreparationRequirement>),
}

#[derive(Default)]
pub struct ValidationReport {
    pub issues: Vec<ConfigurationIssue>,
    pub unavailable: Vec<ConfigurationIssue>,
    pub warnings: Vec<ConfigurationIssue>,
    pub requirements: Vec<PreparationRequirement>,
    pub bindings: BTreeMap<String, UserBinding>,
    pub node_bindings: BTreeMap<NodeId, UserBinding>,
}
type Check = ValidationReport;
impl ValidationReport {
    /// A dependency the configuration needs. Retained from the previous
    /// version it is accepted as inert; newly introduced it is an error.
    pub fn dependency(&mut self, issue: ConfigurationIssue, retained: bool) {
        if retained {
            self.unavailable.push(issue);
        } else {
            self.issues.push(issue);
        }
    }
    pub fn issue(&mut self, issue: ConfigurationIssue) {
        self.issues.push(issue);
    }
    /// Accepted configuration that is probably not what the author meant.
    pub fn warn(&mut self, issue: ConfigurationIssue) {
        self.warnings.push(issue);
    }
    pub fn require(&mut self, path: impl Into<String>, kind: PreparationKind) {
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
fn transport_issue(
    path: impl Into<String>,
    error: &meshway_transport_spec::DeclarationError,
) -> ConfigurationIssue {
    ConfigurationIssue::new(path, IssueCode::InvalidTransport).with("reason", error.code())
}

/// Policy hooks run within normalization, before final error/preparation
/// selection. This preserves author-order field pointers and dependency binding
/// while keeping rule semantics outside the typed configuration crate.
pub trait ConfigurationSemantics {
    fn access_and_topology(
        &self,
        configuration: &mut NetworkConfiguration,
        context: &ValidationContext<'_>,
        report: &mut ValidationReport,
    );
    fn routing(
        &self,
        configuration: &mut NetworkConfiguration,
        context: &ValidationContext<'_>,
        report: &mut ValidationReport,
    );
}

/// Retained but withdrawn dependencies are accepted configuration, never
/// executable permission. Consumers must fail closed for these routes and
/// still check live identity/inventory/publication dependencies when compiling.
pub struct NormalizedConfiguration {
    pub configuration: NetworkConfiguration,
    pub user_bindings: BTreeMap<String, UserBinding>,
    pub node_user_bindings: BTreeMap<NodeId, UserBinding>,
    pub unavailable_dependencies: Vec<ConfigurationIssue>,
    /// Accepted as written, but probably not intended (see `IssueCode`).
    pub warnings: Vec<ConfigurationIssue>,
}

pub fn normalize_with_semantics(
    input: NetworkConfigurationInput,
    context: &ValidationContext<'_>,
    semantics: &impl ConfigurationSemantics,
) -> Result<NormalizedConfiguration, ConfigurationError> {
    normalize_inner(input, context, false, semantics).map(|(normalized, _, _)| normalized)
}

/// Checked, non-executable declarations for a read-only preview. Automatic
/// material remains absent: this type cannot be passed to the execution compiler.
pub struct ConfigurationPlan {
    pub input: NetworkConfigurationInput,
    pub view: ConfigurationView,
    pub user_bindings: BTreeMap<String, UserBinding>,
    pub node_user_bindings: BTreeMap<NodeId, UserBinding>,
    pub unavailable_dependencies: Vec<ConfigurationIssue>,
    pub warnings: Vec<ConfigurationIssue>,
    pub requirements: Vec<PreparationRequirement>,
}

pub fn plan_with_semantics(
    input: NetworkConfigurationInput,
    context: &ValidationContext<'_>,
    semantics: &impl ConfigurationSemantics,
) -> Result<ConfigurationPlan, ConfigurationError> {
    let (normalized, pending, requirements) = normalize_inner(input, context, true, semantics)?;
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
        warnings: normalized.warnings,
    })
}

type PendingListeners = BTreeMap<(NodeId, String), Listener>;
fn normalize_inner(
    mut input: NetworkConfigurationInput,
    context: &ValidationContext<'_>,
    planning: bool,
    semantics: &impl ConfigurationSemantics,
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
        check.issue(transport_issue("/defaults/dialers", &e));
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
            check.issue(ConfigurationIssue::new(&path, IssueCode::UnknownNode).with("node", id));
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
        if !valid_node_hostname(&node.hostname) {
            check.issue(ConfigurationIssue::new(
                format!("{path}/hostname"),
                IssueCode::InvalidHostname,
            ));
        }
        if let Input::Value(dialers) = &node.dialers
            && let Err(e) = dialers.validate()
        {
            check.issue(transport_issue(format!("{path}/dialers"), &e));
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
                check.issue(ConfigurationIssue::new(&lp, IssueCode::EmptyListenerKey));
            }
            if !context.recreated_listeners.contains(&(id, name.clone()))
                && let Some(previous) = old.and_then(|n| n.listeners.get(&name))
            {
                inherit_materials(&mut listener, previous.declaration());
            }
            if let Err(e) = listener.normalize() {
                check.issue(transport_issue(&lp, &e));
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
            match PreparedListener::prepare(listener) {
                Ok(value) => {
                    listeners.insert(name, value);
                }
                Err(e) => check.issue(transport_issue(&lp, &e)),
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
    semantics.access_and_topology(&mut config, context, &mut check);
    references::dns(&mut config, &mut check);
    semantics.routing(&mut config, context, &mut check);
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
            warnings: check.warnings,
        },
        pending,
        requirements,
    ))
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
        // A managed certificate is issued for one server name; a different
        // name gets a new one instead of inheriting a certificate that would
        // not verify. Provided material is the author's to keep or replace.
        let same_managed_name = t.mode != meshway_transport_spec::TlsMode::Managed
            || t.server_name.eq_ignore_ascii_case(&p.server_name);
        if t.mode == p.mode && same_managed_name {
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
