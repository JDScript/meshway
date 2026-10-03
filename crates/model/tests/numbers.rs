use meshway_model::{Count, InvalidNumber, MAX_SAFE_INTEGER, Revision, Sequence, Version};

macro_rules! check_number {
    ($test:ident, $type:ty, $minimum:expr) => {
        #[test]
        fn $test() {
            for raw in [$minimum, 1, 42, MAX_SAFE_INTEGER - 1, MAX_SAFE_INTEGER] {
                let value = <$type>::new(raw).unwrap();
                assert_eq!(<$type>::try_from(raw).unwrap(), value);
                assert_eq!(value.get(), raw);
                assert_eq!(u64::from(value), raw);
                assert_eq!(value.to_string(), raw.to_string());
                let encoded = serde_json::to_string(&value).unwrap();
                assert_eq!(encoded, raw.to_string());
                assert_eq!(serde_json::from_str::<$type>(&encoded).unwrap(), value);
                if raw < MAX_SAFE_INTEGER {
                    let next = value.checked_next().unwrap();
                    assert!(next > value);
                    assert_eq!(next.get(), raw + 1);
                }
            }
            let maximum = <$type>::MAX;
            assert_eq!(maximum.checked_next(), Err(InvalidNumber::Overflow));
            assert_eq!(maximum.get(), MAX_SAFE_INTEGER);
            assert_eq!(<$type>::MIN.get(), $minimum);
            for raw in [MAX_SAFE_INTEGER + 1, u64::MAX] {
                assert_eq!(
                    <$type>::new(raw),
                    Err(InvalidNumber::OutOfRange { minimum: $minimum })
                );
                assert!(<$type>::try_from(raw).is_err());
            }
            for invalid in [
                "-1",
                "-0",
                "0.5",
                "1.5",
                "1.0",
                "1e0",
                "9007199254740992",
                "18446744073709551615",
                "18446744073709551616",
                "1e100",
                "\"1\"",
                "null",
                "true",
                "[]",
                "{}",
            ] {
                assert!(serde_json::from_str::<$type>(invalid).is_err(), "{invalid}");
            }
        }
    };
}

check_number!(revision, Revision, 1);
check_number!(version, Version, 1);
check_number!(sequence, Sequence, 0);
check_number!(count, Count, 0);

#[test]
fn revision_and_version_exclude_zero() {
    assert!(Revision::new(0).is_err());
    assert!(Version::new(0).is_err());
    assert!(serde_json::from_str::<Revision>("0").is_err());
    assert!(serde_json::from_str::<Version>("0").is_err());
    assert_eq!(
        serde_json::from_str::<Sequence>("0").unwrap(),
        Sequence::MIN
    );
}

#[test]
fn order_is_numeric_including_the_exact_json_precision_boundary() {
    let mut revisions = [
        Revision::MAX,
        Revision::new(10).unwrap(),
        Revision::new(MAX_SAFE_INTEGER - 1).unwrap(),
        Revision::new(2).unwrap(),
    ];
    revisions.sort();
    assert_eq!(
        revisions.map(Revision::get),
        [2, 10, MAX_SAFE_INTEGER - 1, MAX_SAFE_INTEGER]
    );
    let json = serde_json::to_string(&revisions).unwrap();
    assert_eq!(json, "[2,10,9007199254740990,9007199254740991]");
    assert_eq!(
        serde_json::from_str::<[Revision; 4]>(&json).unwrap(),
        revisions
    );
}
