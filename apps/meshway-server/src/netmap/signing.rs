//! The Server's netmap signing keys. Every sync answer is signed over
//! `netmap_lease_transcript(network, node, lease, server, keyId, keyGeneration)`; the
//! public half of the current key reaches the Node in its enrollment response.
//!
//! The keys in effect are read from storage inside the transaction of each
//! answer, so enrollment, rotation and sync agree on one stored fact. While a
//! rotation window is open the answer is signed by the key being replaced and
//! carries the successor, its endorsement by the replaced key and a second
//! signature of the same lease by the successor.
use crate::error::Error;
use crate::secrets::Sealer;
use crate::store::netmap as store;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::{Signer as _, SigningKey};
use meshway_model::{KeyId, NetmapLease, NetworkId, NodeId};
use meshway_protocol::netmap::{
    NetmapKeyRotation, NetmapSignature, NetmapSigner, ServerSigningKey,
    netmap_key_rotation_transcript, netmap_lease_transcript,
};
use rusqlite::Transaction;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub use crate::store::netmap::RotatedSigningKey;

/// Shortest accepted rotation window: one sync hold with its jitter.
pub const MIN_ROTATION_WINDOW: Duration = Duration::from_secs(60);
/// Longest accepted rotation window.
pub const MAX_ROTATION_WINDOW: Duration = Duration::from_secs(30 * 24 * 60 * 60);
/// Default window: twice the longest lease, so a Node that synced at all
/// within its last lease sees the handover.
pub const DEFAULT_ROTATION_WINDOW: Duration = Duration::from_secs(48 * 60 * 60);

/// Decrypted keys are held in memory only, by immutable key ID; no Debug,
/// Display or serialization.
pub(crate) struct NetmapSigningKeys {
    sealer: Arc<Sealer>,
    opened: Mutex<BTreeMap<KeyId, Arc<SigningKey>>>,
}

fn generate() -> Result<zeroize::Zeroizing<[u8; 32]>, Error> {
    let mut seed = zeroize::Zeroizing::new([0; 32]);
    getrandom::fill(seed.as_mut_slice())
        .map_err(|_| Error::Unavailable(crate::error::Unavailability::Entropy))?;
    Ok(seed)
}

fn public_of(seed: &[u8; 32]) -> [u8; 32] {
    SigningKey::from_bytes(seed).verifying_key().to_bytes()
}

/// Creates the first key on first start and checks the keys in effect open.
pub(crate) fn ensure(tx: &Transaction<'_>, sealer: &Sealer) -> Result<(), Error> {
    store::ensure_signing_key(
        tx,
        sealer,
        crate::store::execution::now()?,
        generate,
        public_of,
    )
}

/// Starts a rotation window of `window`; rejected while one is open.
pub(crate) fn rotate(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    window: Duration,
) -> Result<RotatedSigningKey, Error> {
    if !(MIN_ROTATION_WINDOW..=MAX_ROTATION_WINDOW).contains(&window) {
        return Err(Error::InvalidRequest);
    }
    let window_ms = i64::try_from(window.as_millis()).map_err(|_| Error::InvalidRequest)?;
    store::rotate_signing_key(
        tx,
        sealer,
        crate::store::execution::now()?,
        window_ms,
        generate,
        public_of,
    )
}

impl NetmapSigningKeys {
    pub(crate) fn new(sealer: Arc<Sealer>) -> Self {
        Self {
            sealer,
            opened: Mutex::default(),
        }
    }

    fn key(&self, row: &store::SigningKeyRow) -> Result<Arc<SigningKey>, Error> {
        if let Ok(opened) = self.opened.lock()
            && let Some(key) = opened.get(&row.public.key_id)
        {
            return Ok(Arc::clone(key));
        }
        let seed = row.seed(&self.sealer, public_of)?;
        let key = Arc::new(SigningKey::from_bytes(&seed));
        if let Ok(mut opened) = self.opened.lock() {
            opened.insert(row.public.key_id, Arc::clone(&key));
        }
        Ok(key)
    }

    /// The signature of `lease` for one Node and, while a window is open, the
    /// handover to the current key.
    pub(crate) fn issue(
        &self,
        tx: &Transaction<'_>,
        network: NetworkId,
        node: NodeId,
        lease: &NetmapLease,
        now: i64,
    ) -> Result<(NetmapSignature, Option<NetmapKeyRotation>), Error> {
        let keys = store::signing_keys(tx, now)?;
        let current = self.key(&keys.current)?;
        let current_signature = sign(&current, &keys.current.public, network, node, lease);
        let Some((rotating, _)) = &keys.rotating else {
            return Ok((current_signature, None));
        };
        let previous = self.key(rotating)?;
        let not_before_ms = u64::try_from(keys.current.created_at)
            .map_err(|_| Error::decode("server_signing_keys.created_at"))?;
        let endorsement = netmap_key_rotation_transcript(
            network,
            rotating.public.key_id,
            rotating.public.key_generation,
            &keys.current.public,
            not_before_ms,
        );
        Ok((
            sign(&previous, &rotating.public, network, node, lease),
            Some(NetmapKeyRotation {
                next: keys.current.public.clone(),
                not_before_ms,
                endorsement: URL_SAFE_NO_PAD.encode(previous.sign(&endorsement).to_bytes()),
                lease_signature: current_signature,
            }),
        ))
    }
}

fn sign(
    key: &SigningKey,
    public: &ServerSigningKey,
    network: NetworkId,
    node: NodeId,
    lease: &NetmapLease,
) -> NetmapSignature {
    let message = netmap_lease_transcript(
        network,
        node,
        lease,
        NetmapSigner::Server,
        public.key_id,
        public.key_generation,
    );
    NetmapSignature {
        key_id: public.key_id,
        key_generation: public.key_generation,
        signature: URL_SAFE_NO_PAD.encode(key.sign(&message).to_bytes()),
    }
}
