use crate::{Digest, KeyId, PublicationId, Revision, Version};
use serde::Serialize;
use utoipa::ToSchema;

meshway_serde::string_enum! {
/// Control signing and dataplane encryption are deliberately different purposes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum KeyPurpose {
    /// Persistent Ed25519 identity used to prove control-plane ownership.
    Control,
    /// Independent X25519 key used for data-plane protection.
    Dataplane,
}
}

meshway_serde::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
/// Cryptographic algorithm of a public Node key, checked against its intended purpose.
pub enum KeyAlgorithm {
    /// Ed25519 signature key for control identity proofs.
    Ed25519,
    /// X25519 key agreement key for data-plane protection.
    X25519,
}
}

meshway_serde::object! {
/// Canonical hex key bytes. Algorithms are checked against their purpose at binding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodePublicKey {
    /// Algorithm defining how the public key bytes are interpreted.
    pub algorithm: KeyAlgorithm,
    /// Canonical lowercase hexadecimal encoding of the 32 public key bytes.
    pub bytes: Digest,
}
}

meshway_serde::object! {
#[derive(Clone, Debug, PartialEq, Eq)]
/// Public key metadata bound by the Server to one Node and purpose.
pub struct BoundNodeKey {
    /// Stable identifier of this bound key.
    pub id: KeyId,
    /// Key generation used to reject retired authentication material.
    pub generation: Version,
    /// Public material for this binding; private key material is never included.
    pub public_key: NodePublicKey,
}
}

meshway_serde::object! {
#[derive(Clone, Debug, PartialEq, Eq)]
/// Exact Node execution target binding source revision, publication and compiled content.
pub struct ConfigurationRef {
    /// Source configuration revision used to compile the target.
    pub revision: Revision,
    /// Publication fixing the target's identity, material and execution dependencies.
    pub execution_publication: PublicationId,
    /// Digest of this Node's complete compiled target, which may differ from other Nodes' digests.
    pub digest: Digest,
}
}
