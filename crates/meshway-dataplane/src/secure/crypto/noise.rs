//! Noise IK remains owned by Snow. Only its completed transport split is used
//! here, because Snow's message API requires disjoint input/output slices.
//! The Noise ChaChaPoly nonce is four zero bytes followed by u64 little-endian;
//! transport AD is empty, exactly as in Snow's stateless transport API.
use super::{Error, MAX_PLAINTEXT};
use crate::PacketBuffer;
use chacha20poly1305::{ChaCha20Poly1305, KeyInit, aead::AeadInPlace};
use zeroize::Zeroizing;

pub(in crate::secure) struct Transport {
    sending: Zeroizing<[u8; 32]>,
    receiving: Zeroizing<[u8; 32]>,
}

impl Transport {
    pub fn finish(mut handshake: snow::HandshakeState) -> Result<Self, Error> {
        Self::split(&mut handshake)
    }

    fn split(handshake: &mut snow::HandshakeState) -> Result<Self, Error> {
        if !handshake.is_handshake_finished() {
            return Err(Error::State);
        }
        let keys = Zeroizing::new(handshake.dangerously_get_raw_split());
        let (sending, receiving) = if handshake.is_initiator() {
            (&keys.0, &keys.1)
        } else {
            (&keys.1, &keys.0)
        };
        Ok(Self {
            sending: Zeroizing::new(*sending),
            receiving: Zeroizing::new(*receiving),
        })
    }

    pub fn encrypt(&self, sequence: u64, buffer: &mut PacketBuffer) -> Result<(), Error> {
        if sequence == u64::MAX {
            return Err(Error::Nonce);
        }
        if buffer.len() > MAX_PLAINTEXT {
            return Err(Error::Unsupported);
        }
        let len = buffer.len();
        buffer.extend(16).map_err(|_| Error::Capacity)?;
        let (plaintext, output_tag) = buffer.payload_mut().split_at_mut(len);
        let tag = ChaCha20Poly1305::new((&*self.sending).into())
            .encrypt_in_place_detached(&nonce(sequence).into(), &[], plaintext)
            .map_err(|_| Error::Crypto)?;
        output_tag.copy_from_slice(&tag);
        Ok(())
    }

    pub fn decrypt(&self, sequence: u64, buffer: &mut PacketBuffer) -> Result<(), Error> {
        if sequence == u64::MAX {
            return Err(Error::Nonce);
        }
        let len = buffer
            .len()
            .checked_sub(16)
            .filter(|len| *len <= MAX_PLAINTEXT)
            .ok_or(Error::Wire)?;
        let (ciphertext, tag) = buffer.payload_mut().split_at_mut(len);
        ChaCha20Poly1305::new((&*self.receiving).into())
            .decrypt_in_place_detached(
                &nonce(sequence).into(),
                &[],
                ciphertext,
                chacha20poly1305::Tag::from_slice(tag),
            )
            .map_err(|_| Error::Crypto)?;
        buffer.truncate(len);
        Ok(())
    }
}

fn nonce(sequence: u64) -> [u8; 12] {
    let mut nonce = [0; 12];
    nonce[4..].copy_from_slice(&sequence.to_le_bytes());
    nonce
}

#[cfg(test)]
mod tests {
    use super::super::{build_with_key, read_handshake, write_handshake};
    use super::*;

    #[test]
    fn completed_split_matches_snow_bytes_both_directions_and_nonce_boundaries() {
        let keypair = || {
            let pair = snow::Builder::new(super::super::NOISE.parse().unwrap())
                .generate_keypair()
                .unwrap();
            (Zeroizing::new(pair.private), pair.public)
        };
        let (a, ap) = keypair();
        let (b, bp) = keypair();
        let mut a = build_with_key(
            a.as_slice().try_into().unwrap(),
            bp.as_slice().try_into().unwrap(),
            b"same IK prologue",
            true,
        )
        .unwrap();
        let mut b = build_with_key(
            b.as_slice().try_into().unwrap(),
            ap.as_slice().try_into().unwrap(),
            b"same IK prologue",
            false,
        )
        .unwrap();
        assert!(Transport::split(&mut a).is_err());
        let init = write_handshake(&mut a, 96).unwrap();
        read_handshake(&mut b, &init, 96, ap.as_slice().try_into().unwrap()).unwrap();
        let response = write_handshake(&mut b, 48).unwrap();
        read_handshake(&mut a, &response, 48, bp.as_slice().try_into().unwrap()).unwrap();
        let ours_a = Transport::split(&mut a).unwrap();
        let ours_b = Transport::split(&mut b).unwrap();
        let snow_a = a.into_stateless_transport_mode().unwrap();
        let snow_b = b.into_stateless_transport_mode().unwrap();
        for (ours, remote, snow) in [(&ours_a, &ours_b, &snow_a), (&ours_b, &ours_a, &snow_b)] {
            for sequence in [0, 1, 255, u32::MAX as u64 + 1, u64::MAX - 1] {
                for len in [0, 32, 1280, MAX_PLAINTEXT] {
                    let plaintext = vec![0x5a; len];
                    let mut expected = vec![0; len + 16];
                    snow.write_message(sequence, &plaintext, &mut expected)
                        .unwrap();
                    let mut buffer = PacketBuffer::from(plaintext.as_slice());
                    let pointer = buffer.as_ptr();
                    ours.encrypt(sequence, &mut buffer).unwrap();
                    assert_eq!(buffer.as_ptr(), pointer);
                    assert_eq!(buffer.payload(), expected);
                    remote.decrypt(sequence, &mut buffer).unwrap();
                    assert_eq!(buffer.as_ptr(), pointer);
                    assert_eq!(buffer.payload(), plaintext);
                }
            }
            let mut packet = PacketBuffer::from(&b"test"[..]);
            assert!(matches!(
                ours.encrypt(u64::MAX, &mut packet),
                Err(Error::Nonce)
            ));
            ours.encrypt(7, &mut packet).unwrap();
            packet[0] ^= 1;
            assert!(remote.decrypt(7, &mut packet).is_err());
        }
    }
}
