//! Private configuration sealing. No key, plaintext, or ciphertext implements
//! Debug/Display or leaves the application's crate-private service boundary.
//!
//! Envelope v1: `MWSC` | version(1) | KeyId(16) | nonce(24) | ciphertext/tag.
//! XChaCha20-Poly1305 authenticates that header plus a domain-separated context.
//! Master key v1: `MWSK` | version(1) | independent random KeyId(16) | key(32).

use chacha20poly1305::{
    KeyInit, XChaCha20Poly1305, XNonce,
    aead::{Aead, Payload},
};
use meshway_model::{KeyId, NetworkId, Revision};
use rusqlite::Connection;
use zeroize::Zeroizing;

use crate::directory::{Directory, DirectoryError};
use crate::store::keys;

mod cursor;
pub(crate) use cursor::CursorScope;

const HEADER: &[u8; 5] = b"MWSC\x01";
const KEY_HEADER: &[u8; 5] = b"MWSK\x01";
const PREFIX: usize = 45;
const DOMAIN: &[u8] = b"meshway/controller/configuration-sealing/v1\0";

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub(crate) enum SealError {
    #[error("protected value has an unsupported format")]
    Format,
    #[error("protected value authentication failed")]
    Authentication,
    #[error("secure random generation failed")]
    Random,
    #[error("protected value encryption failed")]
    Encryption,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum KeyError {
    #[error("private master key is missing")]
    Missing,
    #[error("private master key format is invalid")]
    Invalid,
    #[error("private master key does not match storage")]
    Mismatch,
    #[error("master key storage operation failed")]
    Database,
    #[error(transparent)]
    Directory(#[from] DirectoryError),
    #[error(transparent)]
    Sealing(#[from] SealError),
}

/// Stable field tags are part of the persisted AAD format.
#[derive(Clone, Copy)]
pub(crate) enum Purpose {
    Source,
    Configuration,
    Bindings,
    Request,
}

pub(crate) struct Sealer {
    id: KeyId,
    key: Zeroizing<[u8; 32]>,
}

pub(crate) struct ProtectedBytes(Vec<u8>);

impl ProtectedBytes {
    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

/// Zeroizes decrypted storage buffers; intentionally has no formatting traits.
pub(crate) struct Plaintext(Zeroizing<Vec<u8>>);

impl Plaintext {
    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl Sealer {
    /// Private deterministic identifiers for revocation/idempotency of low
    /// entropy secrets. These identifiers never appear in public API views.
    pub(crate) fn fingerprint(
        &self,
        network: NetworkId,
        kind: &str,
        value: &[u8],
    ) -> Result<[u8; 32], SealError> {
        use hmac::{Hmac, Mac};
        let mut mac = <Hmac<sha2::Sha256> as Mac>::new_from_slice(self.key.as_ref())
            .map_err(|_| SealError::Encryption)?;
        mac.update(b"meshway/credential-fingerprint/v1\0");
        mac.update(network.as_uuid().as_bytes());
        mac.update(&(kind.len() as u64).to_be_bytes());
        mac.update(kind.as_bytes());
        mac.update(value);
        Ok(mac.finalize().into_bytes().into())
    }
    pub(crate) fn seal_enrollment(
        &self,
        id: uuid::Uuid,
        plaintext: &[u8],
    ) -> Result<ProtectedBytes, SealError> {
        let mut context = b"enrollment/v1\0".to_vec();
        context.extend_from_slice(id.as_bytes());
        self.encrypt(&context, plaintext)
    }
    pub(crate) fn open_enrollment(
        &self,
        id: uuid::Uuid,
        protected: &[u8],
    ) -> Result<Plaintext, SealError> {
        let mut context = b"enrollment/v1\0".to_vec();
        context.extend_from_slice(id.as_bytes());
        self.decrypt(&context, protected)
    }
    pub(crate) fn private_digest(&self, domain: &[u8], bytes: &[u8]) -> [u8; 32] {
        // HMAC-SHA256, key separation via a framed domain. The master key never
        // becomes a public plain digest, even for low-entropy protocol passwords.
        use hmac::{Hmac, Mac};
        let mut mac = <Hmac<sha2::Sha256> as Mac>::new_from_slice(self.key.as_ref())
            .expect("HMAC accepts a 32-byte key");
        mac.update(&(domain.len() as u64).to_be_bytes());
        mac.update(domain);
        mac.update(bytes);
        mac.finalize().into_bytes().into()
    }
    pub(crate) fn installation_id(&self) -> KeyId {
        self.id
    }
    pub(crate) fn seal_execution(
        &self,
        network: NetworkId,
        publication: meshway_model::PublicationId,
        plaintext: &[u8],
    ) -> Result<ProtectedBytes, SealError> {
        let mut context = b"execution-publication/v1\0".to_vec();
        context.extend_from_slice(network.as_uuid().as_bytes());
        context.extend_from_slice(publication.as_uuid().as_bytes());
        self.encrypt(&context, plaintext)
    }
    pub(crate) fn open_execution(
        &self,
        network: NetworkId,
        publication: meshway_model::PublicationId,
        protected: &[u8],
    ) -> Result<Plaintext, SealError> {
        let mut context = b"execution-publication/v1\0".to_vec();
        context.extend_from_slice(network.as_uuid().as_bytes());
        context.extend_from_slice(publication.as_uuid().as_bytes());
        self.decrypt(&context, protected)
    }
    pub(crate) fn seal_netmap(
        &self,
        network: NetworkId,
        node: meshway_model::NodeId,
        serial: meshway_model::Version,
        plaintext: &[u8],
    ) -> Result<ProtectedBytes, SealError> {
        self.encrypt(&netmap_context(network, node, serial), plaintext)
    }
    pub(crate) fn open_netmap(
        &self,
        network: NetworkId,
        node: meshway_model::NodeId,
        serial: meshway_model::Version,
        protected: &[u8],
    ) -> Result<Plaintext, SealError> {
        self.decrypt(&netmap_context(network, node, serial), protected)
    }
    pub(crate) fn seal(
        &self,
        network: NetworkId,
        revision: Revision,
        purpose: Purpose,
        plaintext: &[u8],
    ) -> Result<ProtectedBytes, SealError> {
        self.encrypt(&context(network, revision, purpose), plaintext)
    }

    pub(crate) fn open(
        &self,
        network: NetworkId,
        revision: Revision,
        purpose: Purpose,
        protected: &[u8],
    ) -> Result<Plaintext, SealError> {
        self.decrypt(&context(network, revision, purpose), protected)
    }

    fn encrypt(&self, context: &[u8], plaintext: &[u8]) -> Result<ProtectedBytes, SealError> {
        let mut nonce = [0; 24];
        getrandom::fill(&mut nonce).map_err(|_| SealError::Random)?;
        let mut envelope = Vec::with_capacity(PREFIX);
        envelope.extend_from_slice(HEADER);
        envelope.extend_from_slice(self.id.as_uuid().as_bytes());
        envelope.extend_from_slice(&nonce);
        let aad = aad(&envelope, context);
        let cipher = XChaCha20Poly1305::new((&*self.key).into());
        let encrypted = cipher
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: plaintext,
                    aad: &aad,
                },
            )
            .map_err(|_| SealError::Encryption)?;
        envelope.extend_from_slice(&encrypted);
        Ok(ProtectedBytes(envelope))
    }

    fn decrypt(&self, context: &[u8], protected: &[u8]) -> Result<Plaintext, SealError> {
        if protected.len() < PREFIX + 16 || &protected[..5] != HEADER {
            return Err(SealError::Format);
        }
        if &protected[5..21] != self.id.as_uuid().as_bytes() {
            return Err(SealError::Authentication);
        }
        let aad = aad(&protected[..PREFIX], context);
        let cipher = XChaCha20Poly1305::new((&*self.key).into());
        let plaintext = cipher
            .decrypt(
                XNonce::from_slice(&protected[21..PREFIX]),
                Payload {
                    msg: &protected[PREFIX..],
                    aad: &aad,
                },
            )
            .map_err(|_| SealError::Authentication)?;
        Ok(Plaintext(Zeroizing::new(plaintext)))
    }

    pub(crate) fn check_binding(&self, connection: &Connection) -> Result<(), KeyError> {
        let binding = keys::binding(connection, self.id).map_err(|_| KeyError::Database)?;
        let Some(binding) = binding else {
            return Err(KeyError::Mismatch);
        };
        if !binding.key_matches
            || !self
                .decrypt(b"master-key-binding\0", &binding.proof)
                .map_err(|_| KeyError::Mismatch)?
                .as_bytes()
                .is_empty()
        {
            return Err(KeyError::Mismatch);
        }
        Ok(())
    }
}

fn context(network: NetworkId, revision: Revision, purpose: Purpose) -> Vec<u8> {
    let mut bytes = b"network-configuration\0".to_vec();
    bytes.extend_from_slice(network.as_uuid().as_bytes());
    bytes.extend_from_slice(&revision.get().to_be_bytes());
    bytes.push(match purpose {
        Purpose::Source => 1,
        Purpose::Configuration => 2,
        Purpose::Bindings => 3,
        Purpose::Request => 4,
    });
    bytes
}

fn netmap_context(
    network: NetworkId,
    node: meshway_model::NodeId,
    serial: meshway_model::Version,
) -> Vec<u8> {
    let mut bytes = b"node-netmap/v1\0".to_vec();
    bytes.extend_from_slice(network.as_uuid().as_bytes());
    bytes.extend_from_slice(node.as_uuid().as_bytes());
    bytes.extend_from_slice(&serial.get().to_be_bytes());
    bytes
}

fn aad(header: &[u8], context: &[u8]) -> Vec<u8> {
    let mut aad = DOMAIN.to_vec();
    aad.extend_from_slice(header);
    aad.extend_from_slice(context);
    aad
}

/// Called inside schema initialization on the SQLite thread with its Directory
/// lifetime guard. `fresh` means the schema transaction started on a truly empty
/// database; an existing committed schema can never recreate or rebind a key.
pub(crate) fn initialize(
    connection: &Connection,
    directory: &Directory,
    fresh: bool,
) -> Result<Sealer, KeyError> {
    let bytes = match directory.read_key(false)? {
        Some(bytes) => bytes,
        None if !fresh => return Err(KeyError::Missing),
        None => match directory.read_key(true)? {
            Some(bytes) => {
                decode_key(&bytes)?;
                directory.publish_key(None)?;
                bytes
            }
            None => {
                let mut bytes = Zeroizing::new(vec![0; 53]);
                bytes[..5].copy_from_slice(KEY_HEADER);
                // Independent random ID, never a public hash of the secret.
                let id = random_key_id()?;
                bytes[5..21].copy_from_slice(id.as_uuid().as_bytes());
                getrandom::fill(&mut bytes[21..]).map_err(|_| SealError::Random)?;
                directory.publish_key(Some(&bytes))?;
                bytes
            }
        },
    };
    let sealer = decode_key(&bytes)?;
    if fresh {
        let proof = sealer.encrypt(b"master-key-binding\0", b"")?;
        keys::insert_binding(connection, sealer.id, &proof).map_err(|_| KeyError::Database)?;
    }
    sealer.check_binding(connection)?;
    Ok(sealer)
}

fn decode_key(bytes: &[u8]) -> Result<Sealer, KeyError> {
    if bytes.len() != 53 || &bytes[..5] != KEY_HEADER {
        return Err(KeyError::Invalid);
    }
    let id = uuid::Uuid::from_slice(&bytes[5..21]).map_err(|_| KeyError::Invalid)?;
    let id = KeyId::new(id).map_err(|_| KeyError::Invalid)?;
    let mut key = Zeroizing::new([0; 32]);
    key.copy_from_slice(&bytes[21..]);
    Ok(Sealer { id, key })
}

fn random_key_id() -> Result<KeyId, SealError> {
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes).map_err(|_| SealError::Random)?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    KeyId::new(uuid::Uuid::from_bytes(bytes)).map_err(|_| SealError::Random)
}

#[cfg(test)]
mod tests;
