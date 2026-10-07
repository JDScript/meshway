use std::hint::black_box;

pub const KEY: [u8; 32] = [0x42; 32];

pub fn nonce(sequence: u64) -> [u8; 12] {
    let mut value = [0; 12];
    value[4..].copy_from_slice(&sequence.to_le_bytes());
    value
}

pub trait Cipher {
    fn new() -> Self;
    fn seal(&self, nonce: [u8; 12], aad: &[u8], bytes: &mut [u8]) -> [u8; 16];
    fn open(&self, nonce: [u8; 12], aad: &[u8], bytes: &mut [u8], tag: [u8; 16]);
}

pub struct Old(chacha_old::ChaCha20Poly1305);
impl Cipher for Old {
    fn new() -> Self {
        use chacha_old::KeyInit;
        Self(chacha_old::ChaCha20Poly1305::new(black_box(&KEY).into()))
    }
    fn seal(&self, nonce: [u8; 12], aad: &[u8], bytes: &mut [u8]) -> [u8; 16] {
        use chacha_old::aead::AeadInPlace;
        self.0
            .encrypt_in_place_detached(&nonce.into(), aad, bytes)
            .expect("seal")
            .into()
    }
    fn open(&self, nonce: [u8; 12], aad: &[u8], bytes: &mut [u8], tag: [u8; 16]) {
        use chacha_old::aead::AeadInPlace;
        self.0
            .decrypt_in_place_detached(&nonce.into(), aad, bytes, &tag.into())
            .expect("open");
    }
}

pub struct New(chacha_new::ChaCha20Poly1305);
impl Cipher for New {
    fn new() -> Self {
        use chacha_new::KeyInit;
        Self(chacha_new::ChaCha20Poly1305::new(black_box(&KEY).into()))
    }
    fn seal(&self, nonce: [u8; 12], aad: &[u8], bytes: &mut [u8]) -> [u8; 16] {
        use chacha_new::AeadInOut;
        self.0
            .encrypt_inout_detached(&nonce.into(), aad, bytes.into())
            .expect("seal")
            .into()
    }
    fn open(&self, nonce: [u8; 12], aad: &[u8], bytes: &mut [u8], tag: [u8; 16]) {
        use chacha_new::AeadInOut;
        self.0
            .decrypt_inout_detached(&nonce.into(), aad, bytes.into(), &tag.into())
            .expect("open");
    }
}

pub struct Ring(ring::aead::LessSafeKey);
impl Cipher for Ring {
    fn new() -> Self {
        Self(ring::aead::LessSafeKey::new(
            ring::aead::UnboundKey::new(&ring::aead::CHACHA20_POLY1305, black_box(&KEY))
                .expect("key"),
        ))
    }
    fn seal(&self, nonce: [u8; 12], aad: &[u8], bytes: &mut [u8]) -> [u8; 16] {
        self.0
            .seal_in_place_separate_tag(
                ring::aead::Nonce::assume_unique_for_key(nonce),
                ring::aead::Aad::from(aad),
                bytes,
            )
            .expect("seal")
            .as_ref()
            .try_into()
            .expect("16-byte tag")
    }
    fn open(&self, nonce: [u8; 12], aad: &[u8], bytes: &mut [u8], tag: [u8; 16]) {
        self.0
            .open_in_place_separate_tag(
                ring::aead::Nonce::assume_unique_for_key(nonce),
                ring::aead::Aad::from(aad),
                tag.into(),
                bytes,
                0..,
            )
            .expect("open");
    }
}

/// Cross-provider equality is checked before timing, including empty-plaintext AAD tags.
pub fn validate(length: usize) {
    let plaintext = vec![0xa5; length];
    for aad in [vec![], vec![0x5a; length]] {
        let mut old = plaintext.clone();
        let mut new = plaintext.clone();
        let mut ring = plaintext.clone();
        let tag = Old::new().seal(nonce(7), &aad, &mut old);
        assert_eq!(New::new().seal(nonce(7), &aad, &mut new), tag);
        assert_eq!(Ring::new().seal(nonce(7), &aad, &mut ring), tag);
        assert_eq!(old, new);
        assert_eq!(old, ring);
        Old::new().open(nonce(7), &aad, &mut old, tag);
        New::new().open(nonce(7), &aad, &mut new, tag);
        Ring::new().open(nonce(7), &aad, &mut ring, tag);
        assert_eq!(old, plaintext);
        assert_eq!(new, plaintext);
        assert_eq!(ring, plaintext);
    }
    let old = Old::new().seal(nonce(8), &plaintext, &mut []);
    assert_eq!(New::new().seal(nonce(8), &plaintext, &mut []), old);
    assert_eq!(Ring::new().seal(nonce(8), &plaintext, &mut []), old);
}
