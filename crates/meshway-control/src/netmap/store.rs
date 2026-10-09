//! `netmap.json`: the verified netmap, its lease, the Server signature over
//! the lease and this Node's acceptance seal.
//!
//! ```json
//! { "format": "meshway-netmap-cache/2",
//!   "netmap": CompiledNetmap, "lease": NetmapLease,
//!   "signature": NetmapSignature,
//!   "seal": { "signer": "node", "keyId": …, "keyGeneration": …, "signature": base64url } }
//! ```
//!
//! Both signatures cover `netmap_lease_transcript` (protocol). The Server
//! signature by the key pinned at enrollment is the authority; the Node seal
//! by its own control key only records that this identity accepted the
//! content. Restart issues a snapshot only when format, content digest,
//! binding, both signatures and an unexpired lease all hold. A format 1 file
//! (Node seal only) is at most a delta base, never authority.
use crate::{Error, identity::Keys, storage};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::{Signature, Signer as _, VerifyingKey};
use meshway_configuration::netmap::CompiledNetmap;
use meshway_model::{KeyAlgorithm, KeyId, NetmapLease, NodePublicKey, Version};
use meshway_protocol::{
    EnrollmentResponse, NetmapSignature, NetmapSigner, netmap_lease_transcript,
};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

pub(super) const FILE: &str = "netmap.json";
const FORMAT: &str = "meshway-netmap-cache/2";
const FORMAT_V1: &str = "meshway-netmap-cache/1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum Signer {
    /// The Node's own registered control key.
    Node,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Seal {
    signer: Signer,
    key_id: KeyId,
    key_generation: Version,
    signature: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Written<'a> {
    format: &'static str,
    netmap: &'a CompiledNetmap,
    lease: &'a NetmapLease,
    signature: &'a NetmapSignature,
    seal: Seal,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Read {
    format: String,
    netmap: CompiledNetmap,
    lease: NetmapLease,
    #[serde(default)]
    signature: Option<NetmapSignature>,
    seal: Seal,
}

/// A loaded cache. Only `authority` may become a snapshot (if unexpired).
pub(super) struct Loaded {
    pub netmap: CompiledNetmap,
    pub lease: NetmapLease,
    pub authority: bool,
}

/// Content, lease and own identity agree; the content digest verifies.
pub(super) fn bound(
    netmap: &CompiledNetmap,
    lease: &NetmapLease,
    binding: &EnrollmentResponse,
) -> bool {
    let content = &netmap.netmap;
    netmap.verify().is_ok()
        && lease.validate().is_ok()
        && lease.netmap == netmap.reference()
        && content.network_id == binding.network_id
        && content.node_id == binding.node_id
        && content.self_.self_keys.control_key == binding.keys.control
        && content.self_.self_keys.dataplane_key == binding.keys.dataplane
}

/// The Server signature of `lease` for this binding, by the pinned key.
/// A different key ID or generation is `ServerKeyMismatch`; no pin, a
/// non-Ed25519 pin or a bad signature is `SignatureInvalid`.
pub(super) fn verify_server(
    binding: &EnrollmentResponse,
    lease: &NetmapLease,
    signature: &NetmapSignature,
) -> Result<(), Error> {
    let pin = binding
        .server_signing_key
        .as_ref()
        .ok_or(Error::SignatureInvalid)?;
    if signature.key_id != pin.key_id || signature.key_generation != pin.key_generation {
        return Err(Error::ServerKeyMismatch);
    }
    let message = netmap_lease_transcript(
        binding.network_id,
        binding.node_id,
        lease,
        NetmapSigner::Server,
        pin.key_id,
        pin.key_generation,
    );
    if !verify(&pin.public_key, &message, &signature.signature) {
        return Err(Error::SignatureInvalid);
    }
    Ok(())
}

fn seal_message(binding: &EnrollmentResponse, lease: &NetmapLease) -> Vec<u8> {
    netmap_lease_transcript(
        binding.network_id,
        binding.node_id,
        lease,
        NetmapSigner::Node,
        binding.keys.control.id,
        binding.keys.control.generation,
    )
}

pub(super) fn encode(
    netmap: &CompiledNetmap,
    lease: &NetmapLease,
    signature: &NetmapSignature,
    binding: &EnrollmentResponse,
    keys: &Keys,
) -> Result<Zeroizing<Vec<u8>>, Error> {
    if !bound(netmap, lease, binding) {
        return Err(Error::Contract);
    }
    verify_server(binding, lease, signature)?;
    let seal = keys
        .signing_key()?
        .sign(&seal_message(binding, lease))
        .to_bytes();
    storage::encode(&Written {
        format: FORMAT,
        netmap,
        lease,
        signature,
        seal: Seal {
            signer: Signer::Node,
            key_id: binding.keys.control.id,
            key_generation: binding.keys.control.generation,
            signature: URL_SAFE_NO_PAD.encode(seal),
        },
    })
}

/// A verified cache for this identity, or `None`. Expiry is not checked here:
/// an expired authoritative cache is still a valid delta base.
pub(super) fn load(
    directory: &meshway_private_storage::Directory,
    binding: &EnrollmentResponse,
) -> Result<Option<Loaded>, Error> {
    let Some(bytes) = storage::read(directory, FILE)? else {
        return Ok(None);
    };
    let Ok(cache) = storage::decode::<Read>(&bytes) else {
        return Ok(None);
    };
    let key = &binding.keys.control;
    let sealed = cache.seal.signer == Signer::Node
        && cache.seal.key_id == key.id
        && cache.seal.key_generation == key.generation
        && verify(
            &key.public_key,
            &seal_message(binding, &cache.lease),
            &cache.seal.signature,
        );
    if !sealed || !bound(&cache.netmap, &cache.lease, binding) {
        return Ok(None);
    }
    let authority = match (cache.format.as_str(), &cache.signature) {
        (FORMAT, Some(signature)) => {
            if verify_server(binding, &cache.lease, signature).is_err() {
                return Ok(None);
            }
            true
        }
        (FORMAT_V1, None) => false,
        _ => return Ok(None),
    };
    Ok(Some(Loaded {
        netmap: cache.netmap,
        lease: cache.lease,
        authority,
    }))
}

fn verify(key: &NodePublicKey, message: &[u8], signature: &str) -> bool {
    let mut bytes = [0; 64];
    if key.algorithm != KeyAlgorithm::Ed25519
        || signature.len() != 86
        || URL_SAFE_NO_PAD.decode_slice(signature, &mut bytes).ok() != Some(64)
    {
        return false;
    }
    VerifyingKey::from_bytes(key.bytes.as_bytes()).is_ok_and(|verifier| {
        verifier
            .verify_strict(message, &Signature::from_bytes(&bytes))
            .is_ok()
    })
}
