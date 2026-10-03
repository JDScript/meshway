use meshway_model::Digest;

#[test]
fn every_byte_round_trips_without_losing_leading_zeroes() {
    for start in 0..=u8::MAX {
        let bytes = std::array::from_fn(|index| start.wrapping_add(index as u8));
        let digest = Digest::from_bytes(bytes);
        let text = digest.to_string();
        assert_eq!(text.len(), 64);
        assert!(
            text.bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        );
        assert_eq!(text.parse::<Digest>().unwrap().into_bytes(), bytes);
        assert_eq!(*digest.as_bytes(), bytes);
        let encoded = serde_json::to_string(&digest).unwrap();
        assert_eq!(encoded, format!("\"{text}\""));
        assert_eq!(serde_json::from_str::<Digest>(&encoded).unwrap(), digest);
    }
    assert_eq!(Digest::from([0; 32]).to_string(), "0".repeat(64));
    assert_eq!(Digest::from([255; 32]).to_string(), "f".repeat(64));
}

#[test]
fn rejects_noncanonical_or_lossy_encodings() {
    for invalid in [
        "".to_owned(),
        "0".repeat(63),
        "0".repeat(65),
        "A".repeat(64),
        "g".repeat(64),
        format!(" {}", "a".repeat(64)),
        format!("{}\n", "a".repeat(64)),
        format!("0x{}", "a".repeat(62)),
        "é".repeat(32),
    ] {
        assert!(invalid.parse::<Digest>().is_err(), "{invalid:?}");
        assert!(serde_json::from_str::<Digest>(&serde_json::to_string(&invalid).unwrap()).is_err());
    }
    for invalid in ["null", "1", "true", "[]", "{}"] {
        assert!(serde_json::from_str::<Digest>(invalid).is_err());
    }
}

#[test]
fn binary_order_matches_fixed_width_hex_order() {
    let mut low = [0; 32];
    low[31] = 255;
    let mut high = [0; 32];
    high[30] = 1;
    let low = Digest::from_bytes(low);
    let high = Digest::from_bytes(high);
    assert!(low < high);
    assert!(low.to_string() < high.to_string());
}
