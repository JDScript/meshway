//! Production adapters share the App's unique core, configuration cache and
//! SQLite executor. Each adapter accepts exactly one authentication family.
use super::{ReportAdmission, StateError, service::*};
use crate::{configuration::SnapshotCache, execution::Execution, secrets::Sealer};
use chrono::{DateTime, Utc};
use meshway_model::{InstanceId, MembershipRole, Revision};
use meshway_protocol::live::*;
use rusqlite::Transaction;
use std::{collections::BTreeSet, sync::Arc};

#[derive(Clone)]
pub(crate) struct Projection {
    pub execution: Execution,
    pub sealer: Arc<Sealer>,
    pub cache: Arc<SnapshotCache>,
    pub boot: InstanceId,
}
pub struct UserProvider(pub(crate) Projection);
pub struct NodeProvider(pub(crate) Projection);
impl Projection {
    fn snapshot(
        &self,
        tx: &Transaction<'_>,
        scope: &super::VisibilityScope,
    ) -> Result<Arc<crate::configuration::Snapshot>, StateError> {
        self.cache
            .load(
                tx,
                scope.network_id,
                scope.configuration_revision,
                &self.sealer,
            )
            .map_err(|_| StateError::Storage)
    }
}
impl StateProvider for UserProvider {
    type Configuration = UserConfigurationView;
    type Entities = UserEntitiesView;
    type Progress = ConfigurationApplyProgress;
    fn can_drain_configuration_changes(&self, view: &ProviderView<Self>) -> bool {
        matches!(view.configuration, UserConfigurationView::Manager { .. })
    }
    fn replay_scope(&self, view: &ProviderView<Self>) -> Result<meshway_model::Digest, StateError> {
        if matches!(view.configuration, UserConfigurationView::Manager { .. }) {
            // A manager may read the whole network; a new execution target,
            // receipt or path is a data change, not loss of this disclosure
            // scope. Observation subscriptions are independently reauthorized.
            let network = &view.entities.network;
            let mut bytes = network.id.as_uuid().as_bytes().to_vec();
            bytes.extend_from_slice(network.membership_id.as_uuid().as_bytes());
            bytes.push(if network.role == MembershipRole::Owner {
                1
            } else {
                2
            });
            Ok(crate::resources::content_digest(&bytes))
        } else {
            disclosure_digest(&view.scope)
        }
    }
    fn same_authority(&self, previous: &ProviderView<Self>, current: &ProviderView<Self>) -> bool {
        previous.entities.network.membership_id == current.entities.network.membership_id
            && previous.entities.network.role == current.entities.network.role
    }
    fn view(
        &self,
        tx: &Transaction<'_>,
        identity: &ReadIdentity,
        now: DateTime<Utc>,
    ) -> Result<ProviderView<Self>, StateError> {
        let (scope, network) = self.authority(tx, identity, now)?;
        let manager = matches!(network.role, MembershipRole::Owner | MembershipRole::Admin);
        let snapshot = self.0.snapshot(tx, &scope)?;
        let cfg = &snapshot.document.configuration;
        let configuration = if manager {
            UserConfigurationView::Manager {
                network_id: network.id,
                revision: snapshot.revision,
                configuration: Box::new(cfg.into()),
            }
        } else {
            UserConfigurationView::Member {
                network_id: network.id,
                revision: snapshot.revision,
                nodes: cfg
                    .nodes
                    .iter()
                    .filter(|(id, _)| scope.full.contains(id))
                    .map(|(id, n)| (*id, n.into()))
                    .collect(),
            }
        };
        let nodes = crate::store::execution::inventory_in_transaction(
            tx,
            network.id,
            cfg,
            &scope.visible,
            manager,
        )
        .map_err(node_error)?
        .items
        .into_iter()
        .map(|node| (node.id, node))
        .collect();
        let members =
            crate::store::networks::member_snapshot(tx, &network, 10_000).map_err(network_error)?;
        if members.len() > 10_000 {
            return Err(StateError::Budget);
        }
        let members = members
            .into_iter()
            .map(|member| (member.membership_id, member))
            .collect::<std::collections::BTreeMap<_, _>>();
        let member_ids = members.keys().copied().collect();
        let progress = Some(super::progress::read(
            tx,
            network.id,
            snapshot.revision,
            &scope.visible,
            manager,
            self.0.boot,
            now,
        )?);
        Ok(ViewContext {
            scope,
            configuration,
            entities: UserEntitiesView {
                network,
                nodes,
                members,
            },
            progress,
            member_ids,
        })
    }
    fn report(
        &self,
        _: &Transaction<'_>,
        _: &str,
        _: DateTime<Utc>,
    ) -> Result<ReportAdmission, StateError> {
        Err(StateError::Forbidden)
    }
    fn filter_notice(
        &self,
        view: &ProviderView<Self>,
        mut notice: meshway_protocol::events::ConfigurationNotice,
    ) -> meshway_protocol::events::ConfigurationNotice {
        if !matches!(view.configuration, UserConfigurationView::Manager { .. }) {
            redact_notice(&mut notice);
        }
        notice
    }
}
impl UserProvider {
    fn authority(
        &self,
        tx: &Transaction<'_>,
        identity: &ReadIdentity,
        now: DateTime<Utc>,
    ) -> Result<(super::VisibilityScope, meshway_model::NetworkSummary), StateError> {
        let ReadIdentity::UserSession { digest, network } = identity else {
            return Err(StateError::Forbidden);
        };
        let session =
            crate::store::authorization::current_session(tx, digest, now.timestamp_millis())
                .map_err(|_| StateError::Unauthorized)?;
        let scope = self
            .0
            .execution
            .state_scope_in_transaction(tx, identity, now)?;
        let network = crate::store::networks::view(
            tx,
            *network,
            session.user.id,
            &self.0.sealer,
            &self.0.cache,
        )
        .map_err(network_error)?;
        Ok((scope, network))
    }
    pub(crate) fn progress(
        &self,
        tx: &Transaction<'_>,
        identity: &ReadIdentity,
        revision: Revision,
        now: DateTime<Utc>,
    ) -> Result<ConfigurationApplyProgress, StateError> {
        let (scope, network) = self.authority(tx, identity, now)?;
        let manager = matches!(network.role, MembershipRole::Owner | MembershipRole::Admin);
        if !manager && revision != scope.configuration_revision {
            return Err(StateError::Forbidden);
        }
        super::progress::read(
            tx,
            scope.network_id,
            revision,
            &scope.visible,
            manager,
            self.0.boot,
            now,
        )
    }
}
impl StateProvider for NodeProvider {
    type Configuration = NodeConfigurationView;
    type Entities = NodeEntitiesView;
    type Progress = ConfigurationApplyProgress;
    fn replay_scope(&self, view: &ProviderView<Self>) -> Result<meshway_model::Digest, StateError> {
        disclosure_digest(&view.scope)
    }
    fn same_authority(&self, previous: &ProviderView<Self>, current: &ProviderView<Self>) -> bool {
        let node = current.configuration.node_id;
        previous.configuration.node_id == node
            && previous
                .scope
                .current
                .get(&node)
                .map(|node| node.instance_id)
                == current
                    .scope
                    .current
                    .get(&node)
                    .map(|node| node.instance_id)
    }
    fn view(
        &self,
        tx: &Transaction<'_>,
        identity: &ReadIdentity,
        now: DateTime<Utc>,
    ) -> Result<ProviderView<Self>, StateError> {
        let ReadIdentity::Node { authorization } = identity else {
            return Err(StateError::Forbidden);
        };
        let report = self
            .0
            .execution
            .report_admission_in_transaction(tx, authorization, now)?;
        let scope = self
            .0
            .execution
            .state_scope_in_transaction(tx, identity, now)?;
        let snapshot = self.0.snapshot(tx, &scope)?;
        let cfg = &snapshot.document.configuration;
        let target = scope
            .current
            .get(&report.node_id)
            .and_then(|node| node.configurations.first())
            .cloned();
        let configuration = NodeConfigurationView {
            network_id: scope.network_id,
            node_id: report.node_id,
            revision: snapshot.revision,
            settings: cfg
                .nodes
                .get(&report.node_id)
                .ok_or(StateError::Unauthorized)?
                .into(),
            target,
        };
        let nodes = crate::store::execution::inventory_in_transaction(
            tx,
            scope.network_id,
            cfg,
            &scope.visible,
            false,
        )
        .map_err(node_error)?
        .items
        .into_iter()
        .map(|node| (node.id, node))
        .collect();
        let progress = Some(super::progress::read(
            tx,
            scope.network_id,
            snapshot.revision,
            &scope.full,
            false,
            self.0.boot,
            now,
        )?);
        Ok(ViewContext {
            scope,
            configuration,
            entities: NodeEntitiesView { nodes },
            progress,
            member_ids: BTreeSet::new(),
        })
    }
    fn report(
        &self,
        tx: &Transaction<'_>,
        authorization: &str,
        now: DateTime<Utc>,
    ) -> Result<ReportAdmission, StateError> {
        self.0
            .execution
            .report_admission_in_transaction(tx, authorization, now)
    }
    fn filter_notice(
        &self,
        _: &ProviderView<Self>,
        mut notice: meshway_protocol::events::ConfigurationNotice,
    ) -> meshway_protocol::events::ConfigurationNotice {
        redact_notice(&mut notice);
        notice
    }
}
fn redact_notice(notice: &mut meshway_protocol::events::ConfigurationNotice) {
    notice.actor = meshway_protocol::events::EventActor::Redacted;
    notice.request_id = None;
    notice.changed_pointers.clear();
    notice.changes_complete = false;
}
/// Keep Execution's stronger configuration/publication authorization digest
/// intact for history cursors and active observation tasks. Stream disclosure
/// can survive an unrelated source revision, but target changes, revocation,
/// address classifications or peer dependencies invalidate restricted replay.
fn disclosure_digest(scope: &super::VisibilityScope) -> Result<meshway_model::Digest, StateError> {
    let current: Vec<_> = scope
        .current
        .iter()
        .map(|(node, current)| {
            (
                node,
                current.instance_id,
                current.revoked,
                &current.configurations,
                &current.execution,
            )
        })
        .collect();
    let bytes = serde_json::to_vec(&(
        scope.network_id,
        &scope.visible,
        &scope.full,
        &scope.observation_paths,
        &scope.mesh_addresses,
        current,
    ))
    .map_err(|_| StateError::Storage)?;
    Ok(crate::resources::content_digest(&bytes))
}
fn network_error(error: crate::networks::NetworkError) -> StateError {
    use crate::networks::NetworkError;
    match error {
        NetworkError::NotFound => StateError::NotFound,
        NetworkError::PermissionDenied => StateError::Forbidden,
        NetworkError::InvalidSession => StateError::Unauthorized,
        _ => StateError::Storage,
    }
}
fn node_error(error: crate::execution::NodeError) -> StateError {
    use crate::execution::NodeError;
    match error {
        NodeError::Unauthorized => StateError::Unauthorized,
        NodeError::Forbidden => StateError::Forbidden,
        NodeError::Conflict => StateError::Conflict,
        NodeError::NotFound => StateError::NotFound,
        _ => StateError::Storage,
    }
}
