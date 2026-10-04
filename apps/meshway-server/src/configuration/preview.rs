//! Non-executable declaration planning. No generated credential, prepared
//! listener attestation or persistent configuration is manufactured here.
use super::*;
use meshway_configuration::{
    ChangeKind, PreparationKind, plan_configuration, planned_configuration_changes,
};
use meshway_protocol::configuration::ConfigurationPreviewResponse;
use meshway_transport_spec::MaterialRequirement;

pub(super) struct RequestInput<'a> {
    pub input: NetworkConfigurationInput,
    pub source: String,
    pub context: TrustedContext<'a>,
    pub new_content: bool,
}

pub(super) fn build(
    current: &Snapshot,
    input: RequestInput<'_>,
    resources: Option<&resources::Preparation>,
    request_id: RequestId,
) -> Result<ConfigurationPreviewResponse, Error> {
    let plan = plan_configuration(input.input, &input.context.validation)?;
    let mut changes = planned_configuration_changes(&current.document.configuration, &plan.input)?;
    let will_generate: Vec<_> = plan
        .requirements
        .iter()
        .filter(|r| {
            matches!(
                r.kind,
                PreparationKind::Material(
                    MaterialRequirement::Uuid
                        | MaterialRequirement::Password
                        | MaterialRequirement::Certificate
                        | MaterialRequirement::PrivateKey
                )
            )
        })
        .cloned()
        .collect();
    for requirement in &will_generate {
        let field = match requirement.kind {
            PreparationKind::Material(MaterialRequirement::Uuid) => "uuid",
            PreparationKind::Material(MaterialRequirement::Password) => "password",
            PreparationKind::Material(MaterialRequirement::Certificate) => "tls/certificate",
            PreparationKind::Material(MaterialRequirement::PrivateKey) => "tls/privateKey",
            _ => unreachable!("generation-only requirements"),
        };
        let path = format!("{}/{field}", requirement.path);
        if let Some(change) = changes.iter_mut().find(|change| change.path == path) {
            // Delete/recreate replaces an old value; omission is a generation
            // plan, not an instruction to remove the resulting credential.
            change.kind = ChangeKind::Changed;
        }
    }
    let bindings = ConfigurationBindings {
        users: plan.user_bindings,
        node_users: plan.node_user_bindings,
        listeners: listener_bindings(
            current,
            plan.input
                .nodes
                .iter()
                .map(|(id, node)| (*id, node.listeners.keys().cloned().collect())),
            &input.context.validation.recreated_listeners,
        )?,
        rule_sets: input.context.rule_sets,
    };
    changes.extend(binding_changes(&current.bindings, &bindings));
    if input.new_content && !changes.iter().any(|c| c.path == "/bindings/ruleSets") {
        changes.push(ConfigurationChange {
            path: "/bindings/ruleSets".into(),
            kind: ChangeKind::Changed,
        });
    }
    let old = &current.document.configuration.nodes;
    let deleted_node_ids = old
        .keys()
        .filter(|id| !plan.view.nodes.contains_key(id))
        .copied()
        .collect();
    let address_changed_node_ids = old
        .iter()
        .filter(|(id, node)| {
            plan.view
                .nodes
                .get(id)
                .is_some_and(|new| new.addresses != node.addresses)
        })
        .map(|(id, _)| *id)
        .collect();
    Ok(ConfigurationPreviewResponse {
        request_id,
        base_revision: current.revision,
        preview: plan.view,
        requirements: plan.requirements,
        will_generate,
        prepared_rule_sets: resources
            .map(|r| r.preview())
            .transpose()?
            .unwrap_or_default(),
        unavailable_dependencies: plan.unavailable_dependencies,
        change_count: u32::try_from(changes.len())
            .map_err(|_| Error::Internal("change count overflow"))?,
        changes,
        source_changed: current.document.source_jsonc.expose() != input.source,
        deleted_node_ids,
        address_changed_node_ids,
    })
}
