//! What a committed transaction tells the in-memory runtime.
//!
//! A use case returns its effects together with its result; the SQLite
//! executor applies them in order on the owning thread only after COMMIT
//! succeeded. A rolled-back transaction applies nothing, and cancelling the
//! HTTP waiter cannot separate a commit from its effects.

use meshway_model::{NetworkId, NodeId, ObservationId, SessionId};

#[derive(Clone, Debug)]
pub enum Effect {
    /// Publish only after permanent credential revocation and configuration
    /// removal commit. Frees volatile report storage; SQL retains revocation.
    NodeRetired {
        network_id: NetworkId,
        node_id: NodeId,
    },
    State {
        network_id: NetworkId,
        node_id: NodeId,
    },
    Configuration {
        notice: meshway_protocol::events::ConfigurationNotice,
    },
    Authorization {
        network_id: NetworkId,
    },
    Progress {
        network_id: NetworkId,
    },
    Observation {
        network_id: NetworkId,
        observation_id: ObservationId,
    },
    ObservationSample {
        network_id: NetworkId,
        observation: Box<meshway_protocol::observations::ObservationTask>,
    },
    Resource {
        network_id: NetworkId,
    },
    /// A user session ended; its observation subscriptions stop. Not journaled.
    SessionRevoked {
        session_id: SessionId,
    },
}

impl Effect {
    /// The network whose journal records this effect, if it is journaled.
    pub fn network(&self) -> Option<NetworkId> {
        match self {
            Self::State { network_id, .. }
            | Self::NodeRetired { network_id, .. }
            | Self::Authorization { network_id }
            | Self::Progress { network_id }
            | Self::Observation { network_id, .. }
            | Self::ObservationSample { network_id, .. }
            | Self::Resource { network_id } => Some(*network_id),
            Self::Configuration { notice } => Some(notice.network_id),
            Self::SessionRevoked { .. } => None,
        }
    }
}

/// Applies committed effects. Implemented by the State runtime; the executor
/// only ever calls it on its own thread after a successful COMMIT.
pub trait EffectSink: Send + Sync {
    fn apply(&self, effect: Effect);
}

/// Discards every effect. For executors that have no runtime, such as the
/// password-reset CLI.
pub struct NoEffects;
impl EffectSink for NoEffects {
    fn apply(&self, _: Effect) {}
}
