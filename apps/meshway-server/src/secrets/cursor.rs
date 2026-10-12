//! Cursor envelopes use a separate AAD namespace; no synthetic network/revision.
use super::{Plaintext, ProtectedBytes, SealError, Sealer};
use meshway_model::{NetworkId, UserId};

#[derive(Clone, Copy)]
pub(crate) enum CursorScope {
    Networks(UserId),
    Members { user: UserId, network: NetworkId },
    JoinLinks { user: UserId, network: NetworkId },
    EnrollmentKeys { user: UserId, network: NetworkId },
    AccountActivity(UserId),
    AccountSessions(UserId),
    NetworkActivity { user: UserId, network: NetworkId },
    Revisions { user: UserId, network: NetworkId },
    ObservationHistory(meshway_model::Digest),
    RuleSetCatalog { user: UserId, network: NetworkId },
    RuleSetEntries { user: UserId, network: NetworkId },
}

impl CursorScope {
    fn context(self) -> Vec<u8> {
        let mut bytes = b"pagination-cursor/v1\0".to_vec();
        match self {
            Self::EnrollmentKeys { user, network } => {
                bytes.push(9);
                bytes.extend_from_slice(user.as_uuid().as_bytes());
                bytes.extend_from_slice(network.as_uuid().as_bytes());
            }
            Self::AccountSessions(user) => {
                bytes.push(7);
                bytes.extend_from_slice(user.as_uuid().as_bytes());
            }
            Self::ObservationHistory(owner) => {
                bytes.push(8);
                bytes.extend_from_slice(owner.to_string().as_bytes());
            }
            Self::Networks(user) => {
                bytes.push(1);
                bytes.extend_from_slice(user.as_uuid().as_bytes());
            }
            Self::Members { user, network } => {
                bytes.push(2);
                bytes.extend_from_slice(user.as_uuid().as_bytes());
                bytes.extend_from_slice(network.as_uuid().as_bytes());
            }
            Self::JoinLinks { user, network } => {
                bytes.push(3);
                bytes.extend_from_slice(user.as_uuid().as_bytes());
                bytes.extend_from_slice(network.as_uuid().as_bytes());
            }
            Self::AccountActivity(user) => {
                bytes.push(4);
                bytes.extend_from_slice(user.as_uuid().as_bytes());
            }
            Self::NetworkActivity { user, network } => {
                bytes.push(5);
                bytes.extend_from_slice(user.as_uuid().as_bytes());
                bytes.extend_from_slice(network.as_uuid().as_bytes());
            }
            Self::RuleSetCatalog { user, network } => {
                bytes.push(10);
                bytes.extend_from_slice(user.as_uuid().as_bytes());
                bytes.extend_from_slice(network.as_uuid().as_bytes());
            }
            Self::RuleSetEntries { user, network } => {
                bytes.push(11);
                bytes.extend_from_slice(user.as_uuid().as_bytes());
                bytes.extend_from_slice(network.as_uuid().as_bytes());
            }
            Self::Revisions { user, network } => {
                bytes.push(6);
                bytes.extend_from_slice(user.as_uuid().as_bytes());
                bytes.extend_from_slice(network.as_uuid().as_bytes());
            }
        }
        bytes
    }
}

impl Sealer {
    pub(crate) fn seal_cursor(
        &self,
        scope: CursorScope,
        bytes: &[u8],
    ) -> Result<ProtectedBytes, SealError> {
        self.encrypt(&scope.context(), bytes)
    }
    pub(crate) fn open_cursor(
        &self,
        scope: CursorScope,
        bytes: &[u8],
    ) -> Result<Plaintext, SealError> {
        self.decrypt(&scope.context(), bytes)
    }
}
