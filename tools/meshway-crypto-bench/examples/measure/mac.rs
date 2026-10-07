use super::{Run, aead::KEY};
use std::hint::black_box;

const DOMAIN: &[u8] = b"meshway/circuit/hop-mac/v5";

pub fn frame(ip_bytes: usize) -> Vec<u8> {
    let mut frame = DOMAIN.to_vec();
    frame.extend_from_slice(&[0x33; 32]);
    frame.push(0);
    frame.extend_from_slice(&1_u64.to_be_bytes());
    frame.extend_from_slice(&u32::try_from(ip_bytes + 16).expect("length").to_be_bytes());
    frame.resize(frame.len() + ip_bytes + 16, 0xa5);
    frame
}

/// Candidate AEAD authenticates the exact serialized v5 frame before the hop tag.
pub fn wire_frame(ip_bytes: usize) -> Vec<u8> {
    let mut frame = b"MWCIR\0\0\x05".to_vec();
    frame.extend_from_slice(&[3, 0]);
    frame.extend_from_slice(&[0x33; 32]);
    frame.extend_from_slice(&1_u64.to_be_bytes());
    frame.extend_from_slice(&u32::try_from(ip_bytes + 16).expect("length").to_be_bytes());
    frame.resize(frame.len() + ip_bytes + 16, 0xa5);
    frame
}

pub fn v5(run: &mut Run) {
    use hmac_new::{KeyInit as _, Mac as _};
    use hmac_old::Mac as _;
    let frame = frame(run.bytes);
    let prefix = DOMAIN.len();
    let parts = [
        &frame[..prefix],
        &frame[prefix..prefix + 32],
        &frame[prefix + 32..prefix + 33],
        &frame[prefix + 33..prefix + 41],
        &frame[prefix + 41..prefix + 45],
        &frame[prefix + 45..],
    ];
    let old = hmac_old::Hmac::<sha_old::Sha256>::new_from_slice(&KEY).expect("key");
    let new = hmac_new::Hmac::<sha_new::Sha256>::new_from_slice(&KEY).expect("key");
    for prekey in [false, true] {
        let suffix = if prekey { "clone" } else { "new" };
        run.time(&format!("hmac0.12.sha0.10.v5.{suffix}"), |_| {
            let mut mac = if prekey {
                black_box(&old).clone()
            } else {
                hmac_old::Hmac::<sha_old::Sha256>::new_from_slice(black_box(&KEY)).expect("key")
            };
            for part in black_box(parts) {
                mac.update(part);
            }
            mac.finalize().into_bytes()
        });
        run.time(&format!("hmac0.13.sha0.11.v5.{suffix}"), |_| {
            let mut mac = if prekey {
                black_box(&new).clone()
            } else {
                hmac_new::Hmac::<sha_new::Sha256>::new_from_slice(black_box(&KEY)).expect("key")
            };
            for part in black_box(parts) {
                mac.update(part);
            }
            mac.finalize().into_bytes()
        });
    }
    let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, &KEY);
    for prekey in [false, true] {
        let suffix = if prekey { "prekey" } else { "new" };
        run.time(&format!("ring.hmac.v5.{suffix}"), |_| {
            let fresh;
            let key = if prekey {
                black_box(&key)
            } else {
                fresh = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, black_box(&KEY));
                &fresh
            };
            let mut context = ring::hmac::Context::with_key(key);
            for part in black_box(parts) {
                context.update(part);
            }
            context.sign()
        });
    }
    run.time("blake3.keyed.v5.tag16", |_| {
        let hash = blake3::keyed_hash(black_box(&KEY), black_box(&frame));
        let tag: [u8; 16] = hash.as_bytes()[..16].try_into().expect("tag");
        tag
    });
}

pub fn measure(run: &mut Run, input: &[u8]) {
    use hmac_new::{KeyInit as _, Mac as _};
    use hmac_old::Mac as _;
    use sha_new::Digest as _;
    use sha_old::Digest as _;
    let old = hmac_old::Hmac::<sha_old::Sha256>::new_from_slice(&KEY).expect("key");
    let new = hmac_new::Hmac::<sha_new::Sha256>::new_from_slice(&KEY).expect("key");
    let ring = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, &KEY);
    let mut a = old.clone();
    a.update(input);
    let mut b = new.clone();
    b.update(input);
    let a = a.finalize().into_bytes();
    let b = b.finalize().into_bytes();
    assert_eq!(a.as_slice(), b.as_slice());
    assert_eq!(a.as_slice(), ring::hmac::sign(&ring, input).as_ref());
    run.time("hmac0.12.sha0.10.new", |_| {
        let mut mac =
            hmac_old::Hmac::<sha_old::Sha256>::new_from_slice(black_box(&KEY)).expect("key");
        mac.update(black_box(input));
        mac.finalize().into_bytes()
    });
    run.time("hmac0.12.sha0.10.clone", |_| {
        let mut mac = black_box(&old).clone();
        mac.update(black_box(input));
        mac.finalize().into_bytes()
    });
    run.time("hmac0.13.sha0.11.new", |_| {
        let mut mac =
            hmac_new::Hmac::<sha_new::Sha256>::new_from_slice(black_box(&KEY)).expect("key");
        mac.update(black_box(input));
        mac.finalize().into_bytes()
    });
    run.time("hmac0.13.sha0.11.clone", |_| {
        let mut mac = black_box(&new).clone();
        mac.update(black_box(input));
        mac.finalize().into_bytes()
    });
    run.time("ring.hmac.new", |_| {
        let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, black_box(&KEY));
        ring::hmac::sign(&key, black_box(input))
    });
    run.time("ring.hmac.prekey", |_| {
        ring::hmac::sign(black_box(&ring), black_box(input))
    });
    run.time("blake3.keyed.tag16", |_| {
        let hash = blake3::keyed_hash(black_box(&KEY), black_box(input));
        let tag: [u8; 16] = hash.as_bytes()[..16].try_into().expect("tag");
        tag
    });
    run.time("sha0.10.digest", |_| {
        sha_old::Sha256::digest(black_box(input))
    });
    run.time("sha0.11.digest", |_| {
        sha_new::Sha256::digest(black_box(input))
    });
}
