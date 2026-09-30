use crate::{Error, MAX_FRAME_SIZE, PrivateKey, Result};
use snow::{Builder, HandshakeState, StatelessTransportState, params::NoiseParams};
use std::fmt;
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::Zeroizing;

const NOISE: &str = "Noise_IK_25519_ChaChaPoly_BLAKE2s";

#[derive(Clone)]
pub struct Identity {
    private: PrivateKey,
    public: [u8; 32],
}

impl fmt::Debug for Identity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Identity([REDACTED])")
    }
}

impl Identity {
    pub fn generate() -> Result<Self> {
        let params: NoiseParams = NOISE.parse().map_err(|_| Error::Protocol)?;
        let pair = Builder::new(params)
            .generate_keypair()
            .map_err(|_| Error::Authentication)?;
        let private = Zeroizing::new(pair.private);
        let bytes: [u8; 32] = private.as_slice().try_into().map_err(|_| Error::Protocol)?;
        Ok(Self::from_private_key(PrivateKey(Zeroizing::new(bytes))))
    }
    pub fn from_private_key(private: PrivateKey) -> Self {
        let secret = StaticSecret::from(*private.0);
        let public = PublicKey::from(&secret).to_bytes();
        Self { private, public }
    }
    pub fn private_key_hex(&self) -> String {
        self.private.to_hex()
    }
    pub fn public_key_hex(&self) -> String {
        hex::encode(self.public)
    }
    pub fn public_key_bytes(&self) -> [u8; 32] {
        self.public
    }

    pub(crate) fn handshake(
        &self,
        remote: Option<&[u8; 32]>,
        prologue: &[u8],
    ) -> Result<HandshakeState> {
        let params = NOISE.parse().map_err(|_| Error::Protocol)?;
        let builder = Builder::new(params)
            .local_private_key(self.private.0.as_ref())
            .prologue(prologue);
        match remote {
            Some(key) => builder.remote_public_key(key).build_initiator(),
            None => builder.build_responder(),
        }
        .map_err(|_| Error::Authentication)
    }
}

pub(crate) fn write_handshake(state: &mut HandshakeState, payload: &[u8]) -> Result<Vec<u8>> {
    let mut output = vec![0; MAX_FRAME_SIZE];
    let size = state
        .write_message(payload, &mut output)
        .map_err(|_| Error::Authentication)?;
    output.truncate(size);
    Ok(output)
}

pub(crate) fn read_handshake(state: &mut HandshakeState, message: &[u8]) -> Result<Vec<u8>> {
    if message.len() > MAX_FRAME_SIZE {
        return Err(Error::Oversize);
    }
    let mut output = vec![0; MAX_FRAME_SIZE];
    let size = state
        .read_message(message, &mut output)
        .map_err(|_| Error::Authentication)?;
    output.truncate(size);
    Ok(output)
}

/// A 128-packet receive window. Authentication succeeds BEFORE state is advanced.
#[derive(Default)]
struct ReplayWindow {
    highest: Option<u64>,
    seen: u128,
}

impl ReplayWindow {
    fn check(&self, nonce: u64) -> Result<()> {
        if let Some(highest) = self.highest
            && nonce <= highest
        {
            let age = highest - nonce;
            if age >= 128 || self.seen & (1_u128 << age) != 0 {
                return Err(Error::Replay);
            }
        }
        Ok(())
    }
    fn commit(&mut self, nonce: u64) {
        match self.highest {
            None => {
                self.highest = Some(nonce);
                self.seen = 1;
            }
            Some(highest) if nonce > highest => {
                let shift = nonce - highest;
                self.seen = if shift >= 128 {
                    1
                } else {
                    (self.seen << shift) | 1
                };
                self.highest = Some(nonce);
            }
            Some(highest) => self.seen |= 1_u128 << (highest - nonce),
        }
    }
}

pub(crate) struct Cipher {
    state: StatelessTransportState,
    next_nonce: u64,
    replay: ReplayWindow,
    pub id: [u8; 32],
}

impl Cipher {
    pub fn finish(state: HandshakeState) -> Result<Self> {
        if !state.is_handshake_finished() {
            return Err(Error::Authentication);
        }
        let id = state
            .get_handshake_hash()
            .try_into()
            .map_err(|_| Error::Protocol)?;
        Ok(Self {
            state: state
                .into_stateless_transport_mode()
                .map_err(|_| Error::Authentication)?,
            next_nonce: 0,
            replay: ReplayWindow::default(),
            id,
        })
    }
    /// Frame is nonce || authenticated ciphertext. Directional Noise keys are distinct.
    pub fn seal(&mut self, plaintext: &[u8]) -> Result<Vec<u8>> {
        if plaintext.len() > MAX_FRAME_SIZE - 24 {
            return Err(Error::Oversize);
        }
        let nonce = self.next_nonce;
        self.next_nonce = nonce.checked_add(1).ok_or(Error::Closed)?;
        let mut output = vec![0; plaintext.len() + 24];
        output[..8].copy_from_slice(&nonce.to_be_bytes());
        let size = self
            .state
            .write_message(nonce, plaintext, &mut output[8..])
            .map_err(|_| Error::Authentication)?;
        output.truncate(size + 8);
        Ok(output)
    }
    pub fn open(&mut self, message: &[u8]) -> Result<Vec<u8>> {
        if message.len() < 24 || message.len() > MAX_FRAME_SIZE {
            return Err(Error::Oversize);
        }
        let nonce = u64::from_be_bytes(message[..8].try_into().map_err(|_| Error::Protocol)?);
        self.replay.check(nonce)?;
        let mut output = vec![0; message.len() - 8];
        let size = self
            .state
            .read_message(nonce, &message[8..], &mut output)
            .map_err(|_| Error::Authentication)?;
        self.replay.commit(nonce);
        output.truncate(size);
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair() -> (Cipher, Cipher) {
        let alice = Identity::generate().unwrap();
        let bob = Identity::generate().unwrap();
        pair_with(&alice, &bob)
    }

    fn pair_with(alice: &Identity, bob: &Identity) -> (Cipher, Cipher) {
        let mut a = alice
            .handshake(Some(&bob.public), b"test/prologue")
            .unwrap();
        let mut b = bob.handshake(None, b"test/prologue").unwrap();
        let first = write_handshake(&mut a, b"setup").unwrap();
        assert_eq!(read_handshake(&mut b, &first).unwrap(), b"setup");
        assert_eq!(b.get_remote_static(), Some(alice.public.as_slice()));
        let second = write_handshake(&mut b, b"ready").unwrap();
        assert_eq!(read_handshake(&mut a, &second).unwrap(), b"ready");
        (Cipher::finish(a).unwrap(), Cipher::finish(b).unwrap())
    }

    #[test]
    fn native_datagram_nonce_replay_window_and_tamper() {
        let (mut a, mut b) = pair();
        let first = a.seal(b"private application bytes").unwrap();
        let second = a.seal(b"second").unwrap();
        let third = a.seal(b"third").unwrap();
        assert!(
            !first
                .windows(25)
                .any(|part| part == b"private application bytes")
        );
        assert_eq!(b.open(&third).unwrap(), b"third");
        assert_eq!(b.open(&first).unwrap(), b"private application bytes");
        assert_eq!(b.open(&first), Err(Error::Replay));
        let mut tampered = second.clone();
        *tampered.last_mut().unwrap() ^= 1;
        assert_eq!(b.open(&tampered), Err(Error::Authentication));
        // A forgery does not consume the legitimate nonce.
        assert_eq!(b.open(&second).unwrap(), b"second");
        let stale = a.seal(b"stale").unwrap();
        for _ in 0..128 {
            let message = a.seal(b"window advance").unwrap();
            b.open(&message).unwrap();
        }
        assert_eq!(b.open(&stale), Err(Error::Replay));
        let fresh = a.seal(b"fresh").unwrap();
        let mut nonce_tamper = fresh.clone();
        nonce_tamper[0] ^= 1;
        assert_eq!(b.open(&nonce_tamper), Err(Error::Authentication));
        assert_eq!(b.open(&fresh).unwrap(), b"fresh");
        let reverse = b.seal(b"reverse").unwrap();
        assert_eq!(a.open(&reverse).unwrap(), b"reverse");
    }

    #[test]
    fn session_restart_path_context_and_wrong_pinned_key() {
        let identity = Identity::generate().unwrap();
        let target = Identity::generate().unwrap();
        let stranger = Identity::generate().unwrap();
        let mut a = identity
            .handshake(Some(&target.public), b"nyc/lax/sha")
            .unwrap();
        let first = write_handshake(&mut a, b"hello").unwrap();
        let mut wrong_path = target.handshake(None, b"nyc/sha").unwrap();
        assert_eq!(
            read_handshake(&mut wrong_path, &first),
            Err(Error::Authentication)
        );
        let mut wrong_key = stranger.handshake(None, b"nyc/lax/sha").unwrap();
        assert_eq!(
            read_handshake(&mut wrong_key, &first),
            Err(Error::Authentication)
        );
        let (mut a, mut b) = pair_with(&identity, &target);
        let old_ciphertext = a.seal(b"old session").unwrap();
        b.open(&old_ciphertext).unwrap();
        // Same persisted static keys, new ephemeral handshake and reset nonce counter.
        let (_, mut restarted) = pair_with(&identity, &target);
        assert_ne!(b.id, restarted.id);
        assert_eq!(restarted.open(&old_ciphertext), Err(Error::Authentication));
    }
}
