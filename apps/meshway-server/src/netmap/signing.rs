//! The Server's netmap signing key. Every sync answer is signed over
//! `netmap_lease_transcript(network, node, lease, server, keyId, keyGeneration)`; the
//! public half reaches the Node in its enrollment response.
use crate::error::Error;
use crate::secrets::Sealer;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::{Signer as _, SigningKey};
use meshway_model::{NetmapLease, NetworkId, NodeId};
use meshway_protocol::netmap::{
    NetmapSignature, NetmapSigner, ServerSigningKey, netmap_lease_transcript,
};
use rusqlite::Transaction;

/// Holds the private key in memory only; no Debug, Display or serialization.
pub(crate) struct NetmapSigningKey {
    public: ServerSigningKey,
    key: SigningKey,
}

impl NetmapSigningKey {
    /// Loads the current key from private storage, creating it on first start.
    pub(crate) fn load_or_create(tx: &Transaction<'_>, sealer: &Sealer) -> Result<Self, Error> {
        let stored = crate::store::netmap::signing_key(
            tx,
            sealer,
            crate::store::execution::now()?,
            || {
                let mut seed = zeroize::Zeroizing::new([0; 32]);
                getrandom::fill(seed.as_mut_slice())
                    .map_err(|_| Error::Unavailable(crate::error::Unavailability::Entropy))?;
                Ok(seed)
            },
            |seed| SigningKey::from_bytes(seed).verifying_key().to_bytes(),
        )?;
        Ok(Self {
            key: SigningKey::from_bytes(&stored.seed),
            public: stored.public,
        })
    }

    pub(crate) fn sign(
        &self,
        network: NetworkId,
        node: NodeId,
        lease: &NetmapLease,
    ) -> NetmapSignature {
        let message = netmap_lease_transcript(
            network,
            node,
            lease,
            NetmapSigner::Server,
            self.public.key_id,
            self.public.key_generation,
        );
        NetmapSignature {
            key_id: self.public.key_id,
            key_generation: self.public.key_generation,
            signature: URL_SAFE_NO_PAD.encode(self.key.sign(&message).to_bytes()),
        }
    }
}
