//! Control is the production issuer of execution authority. Snapshots are
//! issued only by `netmap` from verified, sealed content and its lease; the
//! packet-plane keys are bound here to exactly such a snapshot.
use crate::{Control, Error};
use meshway_authority::{DataPlaneIdentity, ExecutionSnapshot};

impl Control {
    /// Packet-plane keys for exactly this owner's current netmap snapshot.
    /// The key comparison against the registered binding happens inside the
    /// identity.
    pub fn dataplane_identity(
        &self,
        snapshot: ExecutionSnapshot,
    ) -> Result<DataPlaneIdentity, Error> {
        snapshot.ensure_current()?;
        if !self.netmap_snapshot()?.same_netmap(&snapshot) {
            return Err(Error::Contract);
        }
        let keys = self.record.keys.as_ref().ok_or(Error::Lifecycle)?;
        Ok(DataPlaneIdentity::new(
            snapshot,
            keys.signing_key()?,
            keys.noise_key()?,
        )?)
    }
}
