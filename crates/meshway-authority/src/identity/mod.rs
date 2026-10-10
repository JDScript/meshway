//! Scoped cryptographic key operations for the Node's packet plane.
use crate::{Error, ExecutionSnapshot};
use ed25519_dalek::{Signer, SigningKey};
use meshway_configuration::execution::TrustedNode;
use meshway_model::{BoundNodeKey, Digest, KeyAlgorithm, NetworkId, NodeId, NodePublicKey};
use std::{fmt, sync::Arc};
use zeroize::Zeroizing;

struct Material {
    signing: SigningKey,
    noise: Zeroizing<[u8; 32]>,
}

/// This Node's registered private packet-plane keys and their public halves.
/// Never serializable or printable. They are not authority: a consumer uses
/// them only for a netmap whose own registered keys they [`match`](Self::matches).
#[derive(Clone)]
pub struct DataPlaneKeys {
    material: Arc<Material>,
    control: NodePublicKey,
    dataplane: NodePublicKey,
}

impl DataPlaneKeys {
    pub fn new(signing: SigningKey, noise: Zeroizing<[u8; 32]>) -> Self {
        let control = NodePublicKey {
            algorithm: KeyAlgorithm::Ed25519,
            bytes: Digest::from_bytes(signing.verifying_key().to_bytes()),
        };
        let dataplane = NodePublicKey {
            algorithm: KeyAlgorithm::X25519,
            bytes: Digest::from_bytes(
                curve25519_dalek::montgomery::MontgomeryPoint::mul_base_clamped(*noise).to_bytes(),
            ),
        };
        Self {
            material: Arc::new(Material { signing, noise }),
            control,
            dataplane,
        }
    }

    /// The netmap registered exactly these public halves for this Node.
    pub fn matches(&self, keys: &TrustedNode) -> bool {
        keys.control_key.public_key == self.control
            && keys.dataplane_key.public_key == self.dataplane
    }

    /// Sign only a packet-plane digest transcript for `node` in `network`
    /// under its registered control `key`, which must be exactly this key.
    pub fn sign_packet(
        &self,
        network: NetworkId,
        node: NodeId,
        key: &BoundNodeKey,
        digest: Digest,
    ) -> Result<[u8; 64], Error> {
        if key.public_key != self.control {
            return Err(Error::Contract);
        }
        let message = packet_proof_message(network, node, key, digest);
        Ok(self.material.signing.sign(&message).to_bytes())
    }

    /// Borrow the registered X25519 seed for the maintained Noise implementation.
    pub fn with_noise_key<R>(&self, action: impl FnOnce(&[u8; 32]) -> R) -> R {
        action(&self.material.noise)
    }
}

impl fmt::Debug for DataPlaneKeys {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DataPlaneKeys").finish_non_exhaustive()
    }
}

/// Key operations tied to the exact live execution handoff. Private material is
/// never serializable or printable. The host and in-process crypto code remain
/// trusted; the Noise callback is not a memory-isolation boundary.
#[derive(Clone)]
pub struct DataPlaneIdentity {
    snapshot: ExecutionSnapshot,
    keys: DataPlaneKeys,
}

impl DataPlaneIdentity {
    /// Bind private keys to a snapshot whose target registered exactly their
    /// public halves. The issuer separately proves the snapshot is its current one.
    pub fn new(snapshot: ExecutionSnapshot, keys: &DataPlaneKeys) -> Result<Self, Error> {
        let (_, _, self_keys) = snapshot.self_binding()?;
        if !keys.matches(self_keys) {
            return Err(Error::Contract);
        }
        snapshot.ensure_current()?;
        Ok(Self {
            snapshot,
            keys: keys.clone(),
        })
    }

    pub fn snapshot(&self) -> &ExecutionSnapshot {
        &self.snapshot
    }

    pub fn keys(&self) -> &DataPlaneKeys {
        &self.keys
    }

    /// Borrow the registered X25519 seed to initialize the maintained Noise
    /// implementation. Every call is checked against the original handoff.
    pub fn with_noise_key<R>(&self, action: impl FnOnce(&[u8; 32]) -> R) -> Result<R, Error> {
        self.snapshot.ensure_current()?;
        let output = action(&self.keys.material.noise);
        self.snapshot.ensure_current()?;
        Ok(output)
    }

    /// Sign only the packet-plane digest transcript, never arbitrary control
    /// authentication bytes. Domain, network, node and exact key generation are
    /// always added by this implementation.
    pub fn sign_packet(&self, digest: Digest) -> Result<[u8; 64], Error> {
        let (network, node, self_keys) = self.snapshot.self_binding()?;
        let message = packet_proof_message(network, node, &self_keys.control_key, digest);
        let signature = self.keys.material.signing.sign(&message).to_bytes();
        self.snapshot.ensure_current()?;
        Ok(signature)
    }
}

/// The one canonical signature transcript for packet-plane hop proofs. This
/// helper encodes context only; callers must obtain the key from their own
/// current execution target and perform normal Ed25519 strict verification.
pub fn packet_proof_message(
    network: NetworkId,
    node: NodeId,
    key: &BoundNodeKey,
    digest: Digest,
) -> Vec<u8> {
    let mut message = Vec::with_capacity(128);
    message.extend_from_slice(b"meshway/packet-proof/v2\0");
    message.extend_from_slice(network.as_uuid().as_bytes());
    message.extend_from_slice(node.as_uuid().as_bytes());
    message.extend_from_slice(key.id.as_uuid().as_bytes());
    message.extend_from_slice(&key.generation.get().to_be_bytes());
    message.extend_from_slice(digest.as_bytes());
    message
}

impl fmt::Debug for DataPlaneIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DataPlaneIdentity")
            .field("snapshot", &self.snapshot)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests;
