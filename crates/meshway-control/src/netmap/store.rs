//! `netmap.json`: the verified netmap, its lease and a seal over both.
//!
//! ```json
//! { "format": "meshway-netmap-cache/1",
//!   "netmap": CompiledNetmap, "lease": NetmapLease,
//!   "seal": { "signer": "node", "keyId": …, "keyGeneration": …, "signature": base64url } }
//! ```
//!
//! The seal is an Ed25519 signature over [`lease_message`]. Today the signer is
//! this Node's registered control key: the Server does not yet sign netmaps,
//! so the seal proves that this identity accepted the content from an
//! authenticated sync response, bound to its network, Node and key generation.
//! A Server signer is a new `signer` variant over the same transcript.
//! Loading verifies format, digest, binding, lease and seal; any failure means
//! the file is neither loaded nor treated as identity.
use crate::{Error, identity::Keys, storage};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::{Signature, Signer as _, VerifyingKey};
use meshway_configuration::netmap::CompiledNetmap;
use meshway_model::{BoundNodeKey, KeyId, NetmapLease, NetworkId, NodeId, Version};
use meshway_protocol::EnrollmentResponse;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

pub(super) const FILE: &str = "netmap.json";
const FORMAT: &str = "meshway-netmap-cache/1";
const DOMAIN: &[u8] = b"meshway/netmap-lease/v1\0";

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
    seal: Seal,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Read {
    format: String,
    netmap: CompiledNetmap,
    lease: NetmapLease,
    seal: Seal,
}

/// Canonical transcript of one netmap lease for one Node identity and signer
/// key: domain, network, Node, revision, serial, content digest, lease
/// interval, signer kind, key ID and generation (fixed-width, big-endian).
pub fn lease_message(
    network: NetworkId,
    node: NodeId,
    lease: &NetmapLease,
    signer: &BoundNodeKey,
) -> Vec<u8> {
    let mut message = Vec::with_capacity(160);
    message.extend_from_slice(DOMAIN);
    message.extend_from_slice(network.as_uuid().as_bytes());
    message.extend_from_slice(node.as_uuid().as_bytes());
    message.extend_from_slice(&lease.netmap.revision.get().to_be_bytes());
    message.extend_from_slice(&lease.netmap.serial.get().to_be_bytes());
    message.extend_from_slice(lease.netmap.digest.as_bytes());
    message.extend_from_slice(&lease.issued_at_ms.to_be_bytes());
    message.extend_from_slice(&lease.expires_at_ms.to_be_bytes());
    message.push(0); // Signer::Node
    message.extend_from_slice(signer.id.as_uuid().as_bytes());
    message.extend_from_slice(&signer.generation.get().to_be_bytes());
    message
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

pub(super) fn encode(
    netmap: &CompiledNetmap,
    lease: &NetmapLease,
    binding: &EnrollmentResponse,
    keys: &Keys,
) -> Result<Zeroizing<Vec<u8>>, Error> {
    if !bound(netmap, lease, binding) {
        return Err(Error::Contract);
    }
    let message = lease_message(
        binding.network_id,
        binding.node_id,
        lease,
        &binding.keys.control,
    );
    let signature = keys.signing_key()?.sign(&message).to_bytes();
    storage::encode(&Written {
        format: FORMAT,
        netmap,
        lease,
        seal: Seal {
            signer: Signer::Node,
            key_id: binding.keys.control.id,
            key_generation: binding.keys.control.generation,
            signature: URL_SAFE_NO_PAD.encode(signature),
        },
    })
}

/// A verified, sealed cache for this identity, or `None`. Expiry is not
/// checked here: an expired cache is still a valid delta base.
pub(super) fn load(
    directory: &meshway_private_storage::Directory,
    binding: &EnrollmentResponse,
) -> Result<Option<(CompiledNetmap, NetmapLease)>, Error> {
    let Some(bytes) = storage::read(directory, FILE)? else {
        return Ok(None);
    };
    let Ok(cache) = storage::decode::<Read>(&bytes) else {
        return Ok(None);
    };
    let key = &binding.keys.control;
    let sealed = cache.format == FORMAT
        && cache.seal.signer == Signer::Node
        && cache.seal.key_id == key.id
        && cache.seal.key_generation == key.generation
        && verify(
            key,
            &lease_message(binding.network_id, binding.node_id, &cache.lease, key),
            &cache.seal.signature,
        );
    Ok((sealed && bound(&cache.netmap, &cache.lease, binding))
        .then_some((cache.netmap, cache.lease)))
}

fn verify(key: &BoundNodeKey, message: &[u8], signature: &str) -> bool {
    let mut bytes = [0; 64];
    if signature.len() != 86 || URL_SAFE_NO_PAD.decode_slice(signature, &mut bytes).ok() != Some(64)
    {
        return false;
    }
    VerifyingKey::from_bytes(key.public_key.bytes.as_bytes()).is_ok_and(|verifier| {
        verifier
            .verify_strict(message, &Signature::from_bytes(&bytes))
            .is_ok()
    })
}
