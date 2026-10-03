use super::{EnrollmentCommit, NodeAdmission};
use crate::{
    configuration::{self, ChangeCause, Principal, SnapshotCache, TrustedContext},
    execution::{
        NodeError,
        configuration::{Participant, validation_context},
    },
    secrets::Sealer,
    store::execution as store,
};
use meshway_configuration::{
    Input, NetworkConfigurationInput, NodeAddressesInput, NodeSettingsInput, allocate_addresses,
};
use meshway_model::{RequestId, Revision};
use rusqlite::Transaction;
use std::{collections::BTreeSet, net::IpAddr, sync::Arc};

pub(crate) struct ConfigurationAdmission {
    pub(crate) sealer: Arc<Sealer>,
    pub(crate) cache: Arc<SnapshotCache>,
}
impl EnrollmentCommit for ConfigurationAdmission {
    fn insert_node(
        &self,
        tx: &Transaction<'_>,
        admission: &NodeAdmission,
    ) -> Result<(), NodeError> {
        let revision: i64 = tx.query_row(
            "SELECT current_revision FROM networks WHERE id=?1 AND deleted_at IS NULL",
            [admission.network.as_uuid().as_bytes().as_slice()],
            |r| r.get(0),
        )?;
        let revision = Revision::new(revision.try_into().map_err(|_| NodeError::Storage)?)
            .map_err(|_| NodeError::Storage)?;
        let current = self
            .cache
            .load(tx, admission.network, revision, &self.sealer)
            .map_err(map_error)?;
        let before = meshway_configuration::parse_jsonc(current.document.source_jsonc.expose())
            .map_err(|_| NodeError::Invalid)?;
        let mut input = NetworkConfigurationInput::from(&current.document.configuration);
        if let Some(node) = input.nodes.get_mut(&admission.node) {
            node.identity = admission.identity.clone();
        } else {
            let mut statement=tx.prepare("SELECT address FROM address_retirements WHERE network_id=?1 AND (release_after IS NULL OR release_after>?2)")?;
            let retired = statement
                .query_map(
                    rusqlite::params![
                        admission.network.as_uuid().as_bytes().as_slice(),
                        store::now()?
                    ],
                    |r| r.get::<_, String>(0),
                )?
                .map(|r| r?.parse::<IpAddr>().map_err(|_| NodeError::Storage))
                .collect::<Result<BTreeSet<_>, _>>()?;
            let hostname = unique_hostname(
                admission.initial.hostname.as_deref().unwrap_or("node"),
                input.nodes.values().map(|n| n.hostname.as_str()),
            )?;
            input.nodes.insert(
                admission.node,
                NodeSettingsInput {
                    hostname,
                    remark: String::new(),
                    identity: admission.identity.clone(),
                    enabled: true,
                    addresses: NodeAddressesInput::default(),
                    listeners: admission.initial.listeners.clone(),
                    dialers: Input::Omitted,
                    exit_enabled: false,
                },
            );
            allocate_addresses(
                &mut input,
                &current.document.configuration,
                &retired,
                &BTreeSet::new(),
            )
            .map_err(|_| NodeError::Invalid)?;
        }
        let mut source_target = before.clone();
        source_target
            .nodes
            .insert(admission.node, input.nodes[&admission.node].clone());
        let source = meshway_configuration::rewrite_source(
            current.document.source_jsonc.expose(),
            &before,
            &source_target,
        )
        .map_err(|_| NodeError::Invalid)?;
        let verified = configuration::materials::prepare_materials(
            tx,
            admission.network,
            &mut input,
            &current,
            &self.sealer,
            true,
        )
        .map_err(map_error)?;
        let mut validation =
            validation_context(tx, &current, &input, Some(admission), false).map_err(map_error)?;
        validation.verified_listeners = verified;
        let prepared = configuration::prepare(
            input,
            source,
            &current,
            &TrustedContext {
                validation,
                rule_sets: current.bindings.rule_sets.clone(),
            },
        )
        .map_err(map_error)?;
        let participant = Participant {
            sealer: &self.sealer,
            admission: Some(admission),
        };
        configuration::apply_in_transaction(
            tx,
            &Principal::Lifecycle(admission.actor),
            current.revision,
            &current,
            &prepared,
            configuration::CommitMetadata {
                cause: ChangeCause::Enrollment,
                request: RequestId::new(admission.request.into_uuid())
                    .map_err(|_| NodeError::Storage)?,
            },
            configuration::CommitRuntime {
                sealer: &self.sealer,
                participant: &participant,
            },
        )
        .map_err(map_error)?;
        Ok(())
    }
}
fn unique_hostname<'a>(
    base: &str,
    existing: impl Iterator<Item = &'a str>,
) -> Result<String, NodeError> {
    let existing: BTreeSet<_> = existing.collect();
    if !existing.contains(base) {
        return Ok(base.to_owned());
    }
    for suffix in 2..=10_001 {
        let suffix = format!("-{suffix}");
        let prefix = &base[..base.len().min(63 - suffix.len())];
        let candidate = format!("{}{suffix}", prefix.trim_end_matches('-'));
        if !existing.contains(candidate.as_str()) {
            return Ok(candidate);
        }
    }
    Err(NodeError::Busy)
}
fn map_error(e: configuration::Error) -> NodeError {
    match e {
        configuration::Error::InvalidSession => NodeError::Unauthorized,
        configuration::Error::PermissionDenied => NodeError::Forbidden,
        configuration::Error::Conflict => NodeError::Conflict,
        configuration::Error::NotFound => NodeError::NotFound,
        configuration::Error::Storage => NodeError::Storage,
        _ => NodeError::Invalid,
    }
}
