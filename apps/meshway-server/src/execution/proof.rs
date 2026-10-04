//! Fixed transcript encoding is exported by the protocol crate. Verification uses
//! dalek strict Ed25519 (including weak-key rejection), never X25519-as-signature.
use crate::error::Error;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::{Signature, VerifyingKey};
use meshway_model::{Digest, KeyAlgorithm, NodePublicKey};
use meshway_protocol::{
    WireSecret,
    node::{NodeChallenge, proof_transcript},
};
use sha2::{Digest as _, Sha256};

pub fn verify(
    key: &NodePublicKey,
    challenge: &NodeChallenge,
    signature: &str,
) -> Result<(), Error> {
    if key.algorithm != KeyAlgorithm::Ed25519 {
        return Err(Error::InvalidCredentials);
    }
    verify_bytes(
        key.bytes.as_bytes(),
        &proof_transcript(challenge),
        signature,
    )
}
pub(crate) fn verify_bytes(key: &[u8; 32], message: &[u8], signature: &str) -> Result<(), Error> {
    let key = VerifyingKey::from_bytes(key).map_err(|_| Error::InvalidCredentials)?;
    if key.is_weak() || signature.len() != 86 {
        return Err(Error::InvalidCredentials);
    }
    let bytes = URL_SAFE_NO_PAD
        .decode(signature)
        .map_err(|_| Error::InvalidCredentials)?;
    let signature = Signature::from_slice(&bytes).map_err(|_| Error::InvalidCredentials)?;
    key.verify_strict(message, &signature)
        .map_err(|_| Error::InvalidCredentials)
}
pub fn request_digest<T: serde::Serialize>(intent: &T) -> Result<Digest, Error> {
    // Canonicalize only an already typed intent. This is not an untyped wire
    // decoder. Generated SDK struct/property ordering cannot affect the proof.
    let mut value = serde_json::to_value(intent).map_err(|_| Error::InvalidRequest)?;
    value.sort_all_objects();
    let bytes = serde_json::to_vec(&value).map_err(|_| Error::InvalidRequest)?;
    let mut hash = Sha256::new();
    hash.update(b"meshway/node-intent/v1\0");
    hash.update(bytes);
    Ok(Digest::from_bytes(hash.finalize().into()))
}
pub(crate) fn token_hash(token: &str) -> Result<[u8; 32], Error> {
    if token.len() != 48 || !token.starts_with("mwns_") {
        return Err(Error::InvalidCredentials);
    }
    let decoded = URL_SAFE_NO_PAD
        .decode(&token[5..])
        .map_err(|_| Error::InvalidCredentials)?;
    if decoded.len() != 32 {
        return Err(Error::InvalidCredentials);
    }
    Ok(Sha256::digest(token.as_bytes()).into())
}
pub(crate) fn issue_token() -> Result<(WireSecret, [u8; 32]), Error> {
    let mut bytes = [0; 32];
    getrandom::fill(&mut bytes).map_err(|_| Error::ENTROPY)?;
    let token = format!("mwns_{}", URL_SAFE_NO_PAD.encode(bytes));
    let digest = token_hash(&token)?;
    Ok((WireSecret::new(token), digest))
}
