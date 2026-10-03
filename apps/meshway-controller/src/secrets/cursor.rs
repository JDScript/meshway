//! Cursor envelopes use a separate AAD namespace; no synthetic network/revision.
use super::{Plaintext, ProtectedBytes, SealError, Sealer};
use meshway_model::{NetworkId, UserId};

#[derive(Clone, Copy)]
pub(crate) enum CursorScope {
    Networks(UserId),
    Members { user: UserId, network: NetworkId },
}

impl CursorScope {
    fn context(self) -> Vec<u8> {
        let mut bytes = b"pagination-cursor/v1\0".to_vec();
        match self {
            Self::Networks(user) => {
                bytes.push(1);
                bytes.extend_from_slice(user.as_uuid().as_bytes());
            }
            Self::Members { user, network } => {
                bytes.push(2);
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
