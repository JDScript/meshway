use crate::{Digest, KeyId, PublicationId, Revision, Version};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Control signing and dataplane encryption are deliberately different purposes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum KeyPurpose {
    Control,
    Dataplane,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum KeyAlgorithm {
    Ed25519,
    X25519,
}

meshway_serde::object! {
/// Canonical hex key bytes. Algorithms are checked against their purpose at binding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodePublicKey {
    pub algorithm: KeyAlgorithm,
    pub bytes: Digest,
}
}

meshway_serde::object! {
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoundNodeKey {
    pub id: KeyId,
    pub generation: Version,
    pub public_key: NodePublicKey,
}
}

meshway_serde::object! {
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigurationRef {
    pub revision: Revision,
    pub execution_publication: PublicationId,
    pub digest: Digest,
}
}
