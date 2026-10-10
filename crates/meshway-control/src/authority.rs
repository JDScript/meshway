//! The packet-plane keys of this identity. They are bound to a netmap only
//! by its consumer's comparison with the netmap's registered public keys.
use crate::{Control, Error};
use meshway_authority::DataPlaneKeys;

impl Control {
    pub fn dataplane_keys(&self) -> Result<DataPlaneKeys, Error> {
        self.ready()?;
        let keys = self.record.keys.as_ref().ok_or(Error::Lifecycle)?;
        Ok(DataPlaneKeys::new(keys.signing_key()?, keys.noise_key()?))
    }
}
