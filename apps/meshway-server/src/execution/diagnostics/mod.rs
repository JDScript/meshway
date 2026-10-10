//! Observation-owned, short-lived execution dependencies. The subscription
//! rows are the only renewable authority; Node instance renewal cannot renew them.
use super::Execution;
use crate::error::Error;
use crate::observations::{
    SubscriberPrincipal,
    service::{NodeObservationAdmission, NodeObservationIdentity},
};
use crate::{observations::TaskAdmission, state::service::ReadIdentity, store::execution as store};
use chrono::{DateTime, Utc};
use meshway_model::*;
use meshway_protocol::observations::{CreateObservation, ObservationPath};
use meshway_protocol::{InstanceRequest, node_observations::*};
use rusqlite::Transaction;
mod netmap;
pub use netmap::{NetmapDiagnostic, NetmapDiagnostics, netmap_diagnostic};

struct DiagnosticSubject {
    network_id: NetworkId,
    observation_authorization_digest: Digest,
    expires_at: DateTime<Utc>,
}

/// Node-store lookups made on behalf of an authenticated user. A Node credential
/// or dependency failure must never read as that user's session being invalid.
fn node_dependency_error(error: Error) -> Error {
    match error {
        Error::InvalidCredentials | Error::InvalidSession => Error::PermissionDenied,
        other => other,
    }
}

impl Execution {
    pub(super) fn diagnostic_node_identity(
        &self,
        tx: &Transaction<'_>,
        token: &str,
        instance: &InstanceRequest,
        now: DateTime<Utc>,
    ) -> Result<NodeObservationIdentity, Error> {
        let principal = self.traversal_principal(tx, token, now)?;
        let report = self.current_node_admission(tx, principal.network, principal.node, now)?;
        if report.instance_id != instance.instance_id
            || report.instance_generation != instance.generation
        {
            return Err(Error::InvalidSession);
        }
        let lease = store::lease_for_node(tx, principal.network, principal.node)?;
        Ok(NodeObservationIdentity {
            principal: SubscriberPrincipal::NodeInstance {
                network: principal.network,
                node: principal.node,
                key: principal.key,
                instance: report.instance_id,
                generation: report.instance_generation,
                boot: self.boot,
            },
            expires_at: DateTime::from_timestamp_millis(lease.expires_at_ms as i64)
                .ok_or(Error::InvalidSession)?
                .min(
                    DateTime::from_timestamp_millis(principal.expires_at_ms as i64)
                        .ok_or(Error::InvalidSession)?,
                ),
        })
    }

    fn diagnostic_node_subject(
        &self,
        tx: &Transaction<'_>,
        principal: &SubscriberPrincipal,
        now: DateTime<Utc>,
    ) -> Result<DiagnosticSubject, Error> {
        let SubscriberPrincipal::NodeInstance {
            network,
            node,
            key,
            instance,
            generation,
            boot,
        } = principal
        else {
            return Err(Error::PermissionDenied);
        };
        if *boot != self.boot {
            return Err(Error::InvalidSession);
        }
        let report = self.current_node_admission(tx, *network, *node, now)?;
        if report.instance_id != *instance || report.instance_generation != *generation {
            return Err(Error::InvalidSession);
        }
        let deadline = store::admission::current_session_deadline(
            tx,
            *network,
            *node,
            self.boot,
            *key,
            now.timestamp_millis(),
        )?
        .ok_or(Error::InvalidSession)?;
        let (revision, _, _) = store::current_configuration(tx, &self.sealer, *network)?;
        let lease = store::lease_for_node(tx, *network, *node)?;
        Ok(DiagnosticSubject {
            network_id: *network,
            observation_authorization_digest: super::proof::request_digest(&(
                "node-observation",
                network,
                revision,
                node,
                key,
                instance,
                generation,
                boot,
            ))?,
            expires_at: DateTime::from_timestamp_millis(lease.expires_at_ms as i64)
                .ok_or(Error::InvalidSession)?
                .min(
                    DateTime::from_timestamp_millis(deadline as i64)
                        .ok_or(Error::InvalidSession)?,
                ),
        })
    }
}
