use crate::Error;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::{Signer, SigningKey};
use meshway_model::{Digest, KeyAlgorithm, NodePublicKey};
use meshway_protocol::{EnrollmentResponse, WireSecret};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Keys {
    control: WireSecret,
    dataplane: WireSecret,
}

impl Keys {
    pub(super) fn generate() -> Result<Self, Error> {
        Ok(Self {
            control: random_secret()?,
            dataplane: random_secret()?,
        })
    }
    pub(super) fn control(&self) -> Result<NodePublicKey, Error> {
        Ok(NodePublicKey {
            algorithm: KeyAlgorithm::Ed25519,
            bytes: Digest::from_bytes(
                SigningKey::from_bytes(&*decode(&self.control)?)
                    .verifying_key()
                    .to_bytes(),
            ),
        })
    }
    pub(super) fn dataplane(&self) -> Result<NodePublicKey, Error> {
        Ok(NodePublicKey {
            algorithm: KeyAlgorithm::X25519,
            bytes: Digest::from_bytes(
                curve25519_dalek::montgomery::MontgomeryPoint::mul_base_clamped(*decode(
                    &self.dataplane,
                )?)
                .to_bytes(),
            ),
        })
    }
    pub(super) fn sign(&self, transcript: &[u8]) -> Result<WireSecret, Error> {
        let signature = SigningKey::from_bytes(&*decode(&self.control)?).sign(transcript);
        Ok(WireSecret::new(
            URL_SAFE_NO_PAD.encode(signature.to_bytes()),
        ))
    }
    pub(super) fn matches(&self, binding: &EnrollmentResponse) -> Result<bool, Error> {
        Ok(binding.keys.control.public_key == self.control()?
            && binding.keys.dataplane.public_key == self.dataplane()?)
    }
}

fn decode(secret: &WireSecret) -> Result<Zeroizing<[u8; 32]>, Error> {
    let mut bytes = Zeroizing::new([0; 32]);
    if secret.expose().len() != 43
        || URL_SAFE_NO_PAD
            .decode_slice(secret.expose(), bytes.as_mut())
            .map_err(|_| Error::Record)?
            != 32
    {
        return Err(Error::Record);
    }
    Ok(bytes)
}
pub(super) fn random_secret() -> Result<WireSecret, Error> {
    let mut bytes = Zeroizing::new([0u8; 32]);
    getrandom::fill(bytes.as_mut()).map_err(|_| Error::Cryptography)?;
    Ok(WireSecret::new(URL_SAFE_NO_PAD.encode(bytes.as_ref())))
}
pub(super) fn challenge(verifier: &WireSecret) -> String {
    use sha2::{Digest as _, Sha256};
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.expose().as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dataplane_public_key_matches_rfc7748_x25519_vector() {
        let scalar: Digest = "77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a"
            .parse()
            .unwrap();
        let keys = Keys {
            control: random_secret().unwrap(),
            dataplane: WireSecret::new(URL_SAFE_NO_PAD.encode(scalar.as_bytes())),
        };
        let expected: Digest = "8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a"
            .parse()
            .unwrap();
        assert_eq!(keys.dataplane().unwrap().bytes, expected);
    }
}
