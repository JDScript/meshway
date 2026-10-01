//! A control-plane connection key, independent of enrollment and lab identities.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::{Zeroize, Zeroizing};

use super::{AgentError, Identity, local};

// Local disk format only. Deliberately no Debug implementation, including on
// the containing store. Raw JSON and decoded secret buffers are also zeroized.
#[derive(PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Key {
    format_version: u32,
    controller: String,
    node_id: String,
    private_key: String,
    public_key: String,
}

impl Drop for Key {
    fn drop(&mut self) {
        self.private_key.zeroize();
    }
}

impl Key {
    fn generate(identity: &Identity) -> Result<Self, AgentError> {
        let node_id = identity.node_id.clone().ok_or(AgentError::Invalid(
            "connection key requires a confirmed node identity",
        ))?;
        let mut bytes = Zeroizing::new([0_u8; 32]);
        getrandom::fill(bytes.as_mut())
            .map_err(|_| AgentError::Invalid("OS randomness unavailable for connection key"))?;
        let secret = StaticSecret::from(*bytes);
        Ok(Self {
            format_version: 1,
            controller: identity.controller.clone(),
            node_id,
            private_key: encode_hex(&bytes[..]),
            public_key: encode_hex(PublicKey::from(&secret).as_bytes()),
        })
    }

    fn validate(&self, identity: &Identity) -> Result<(), AgentError> {
        if self.format_version != 1
            || self.controller != identity.controller
            || Some(&self.node_id) != identity.node_id.as_ref()
            || !local::valid_credential(&self.private_key)
            || !meshway_protocol::valid_connection_public_key(&self.public_key)
        {
            return Err(AgentError::Invalid(
                "invalid connection key or controller/node binding; refusing to replace",
            ));
        }
        let mut bytes = Zeroizing::new([0_u8; 32]);
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&self.private_key[index * 2..index * 2 + 2], 16)
                .map_err(|_| AgentError::Invalid("invalid private connection key encoding"))?;
        }
        let secret = StaticSecret::from(*bytes);
        if encode_hex(PublicKey::from(&secret).as_bytes()) != self.public_key {
            return Err(AgentError::Invalid(
                "connection private/public key mismatch; refusing to replace",
            ));
        }
        Ok(())
    }
}

fn encode_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut value = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(value, "{byte:02x}").expect("writing to a String");
    }
    value
}

pub(super) fn path(state: &Path) -> PathBuf {
    let mut value = state.as_os_str().to_owned();
    value.push(".connection-key.json");
    PathBuf::from(value)
}

fn load(state: &Path, identity: &Identity) -> Result<Option<Key>, AgentError> {
    let path = path(state);
    match std::fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
        Ok(_) => {}
    }
    let bytes = Zeroizing::new(local::read_private_bounded(&path, 8192)?);
    let key: Key = serde_json::from_str(&bytes).map_err(|_| {
        AgentError::Invalid("unrecognized or damaged connection key; refusing to replace")
    })?;
    key.validate(identity)?;
    Ok(Some(key))
}

pub(super) struct KeyStore {
    key: Option<Key>,
}

impl KeyStore {
    pub(super) fn load(state: &Path, identity: &Identity) -> Result<Self, AgentError> {
        Ok(Self {
            key: load(state, identity)?,
        })
    }

    pub(super) fn public_key(&self) -> Option<&str> {
        self.key.as_ref().map(|key| key.public_key.as_str())
    }

    /// Caller holds the existing Agent state lock. Never rewrite enrollment.
    pub(super) fn check(&self, state: &Path, identity: &Identity) -> Result<(), AgentError> {
        let parent = state
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        local::private_dir(parent)?;
        if Identity::load(state)? != *identity || load(state, identity)? != self.key {
            return Err(AgentError::Invalid(
                "identity or connection key changed; refusing to publish or acknowledge",
            ));
        }
        Ok(())
    }

    /// The returned public key always has confirmed private-key durability.
    pub(super) fn prepare(
        &mut self,
        state: &Path,
        identity: &Identity,
    ) -> Result<&str, AgentError> {
        self.check(state, identity)?;
        local::ensure_durable(state)?;
        if self.key.is_none() {
            let key = Key::generate(identity)?;
            let bytes = Zeroizing::new(
                serde_json::to_vec(&key)
                    .map_err(|_| AgentError::Invalid("cannot encode connection key"))?,
            );
            let result = local::atomic_write(&path(state), &bytes, false);
            if result.is_ok() || result.as_ref().is_err_and(|error| error.committed()) {
                // A visible key is immutable even if directory fsync failed.
                // Retry confirms durability of this same key, including restart.
                self.key = Some(key);
            }
            result?;
        }
        local::ensure_durable(&path(state))?;
        Ok(self.public_key().expect("persisted connection key"))
    }
}
