use super::*;

#[test]
fn protected_types_cannot_be_formatted_into_ordinary_logs() {
    // Adding Debug/Display to these types makes trait inference ambiguous and
    // this compile-time guard fails, without printing any sensitive fixture.
    trait WithoutDebug<A> {
        fn verify() {}
    }
    impl<T: ?Sized> WithoutDebug<()> for T {}
    impl<T: ?Sized + std::fmt::Debug> WithoutDebug<u8> for T {}
    trait WithoutDisplay<A> {
        fn verify() {}
    }
    impl<T: ?Sized> WithoutDisplay<()> for T {}
    impl<T: ?Sized + std::fmt::Display> WithoutDisplay<u8> for T {}
    <Sealer as WithoutDebug<_>>::verify();
    <Plaintext as WithoutDebug<_>>::verify();
    <ProtectedBytes as WithoutDebug<_>>::verify();
    <Sealer as WithoutDisplay<_>>::verify();
    <Plaintext as WithoutDisplay<_>>::verify();
    <ProtectedBytes as WithoutDisplay<_>>::verify();
}

fn sealer() -> Sealer {
    let mut bytes = Zeroizing::new(vec![0; 53]);
    bytes[..5].copy_from_slice(KEY_HEADER);
    bytes[5..21].copy_from_slice(random_key_id().unwrap().as_uuid().as_bytes());
    getrandom::fill(&mut bytes[21..]).unwrap();
    decode_key(&bytes).unwrap()
}

#[test]
fn authenticated_envelopes_reject_every_byte_mutation_and_context_substitution() {
    let sealer = sealer();
    let network = NetworkId::new(random_key_id().unwrap().into_uuid()).unwrap();
    let other = NetworkId::new(random_key_id().unwrap().into_uuid()).unwrap();
    let revision = Revision::MIN;
    let plaintext = b"private sourceJsonc, protocol password and bindings";
    for purpose in [Purpose::Source, Purpose::Configuration, Purpose::Bindings] {
        let protected = sealer.seal(network, revision, purpose, plaintext).unwrap();
        let again = sealer.seal(network, revision, purpose, plaintext).unwrap();
        assert!(protected.as_bytes() != again.as_bytes());
        assert!(
            !protected
                .as_bytes()
                .windows(plaintext.len())
                .any(|w| w == plaintext)
        );
        assert!(
            sealer
                .open(network, revision, purpose, protected.as_bytes())
                .unwrap()
                .as_bytes()
                == plaintext
        );
        for i in 0..protected.as_bytes().len() {
            let mut bad = protected.as_bytes().to_vec();
            bad[i] ^= 1;
            assert!(sealer.open(network, revision, purpose, &bad).is_err());
        }
        for length in 0..protected.as_bytes().len() {
            assert!(
                sealer
                    .open(network, revision, purpose, &protected.as_bytes()[..length])
                    .is_err()
            );
        }
        assert!(
            sealer
                .open(other, revision, purpose, protected.as_bytes())
                .is_err()
        );
        assert!(
            sealer
                .open(
                    network,
                    revision.checked_next().unwrap(),
                    purpose,
                    protected.as_bytes()
                )
                .is_err()
        );
        for different in [Purpose::Source, Purpose::Configuration, Purpose::Bindings] {
            if context(network, revision, purpose) != context(network, revision, different) {
                assert!(
                    sealer
                        .open(network, revision, different, protected.as_bytes())
                        .is_err()
                );
            }
        }
        let mut wrong_secret = Sealer {
            id: sealer.id,
            key: Zeroizing::new(*sealer.key),
        };
        wrong_secret.key[0] ^= 1;
        assert!(
            wrong_secret
                .open(network, revision, purpose, protected.as_bytes())
                .is_err()
        );
        assert!(
            sealer
                .decrypt(b"master-key-binding\0", protected.as_bytes())
                .is_err()
        );
    }
}

#[test]
fn empty_plaintext_roundtrips_and_errors_are_safe_categories() {
    let sealer = sealer();
    let network = NetworkId::new(random_key_id().unwrap().into_uuid()).unwrap();
    let bytes = sealer
        .seal(network, Revision::MAX, Purpose::Source, b"")
        .unwrap();
    assert!(
        sealer
            .open(network, Revision::MAX, Purpose::Source, bytes.as_bytes())
            .unwrap()
            .as_bytes()
            .is_empty()
    );
    for error in [
        SealError::Format,
        SealError::Authentication,
        SealError::Random,
        SealError::Encryption,
    ] {
        let text = format!("{error:?} {error}");
        assert!(!text.contains(&sealer.id.to_string()));
    }
}

#[test]
fn cursor_scope_is_independent_of_configuration_and_binds_user_and_network() {
    use meshway_model::UserId;
    let sealer = sealer();
    let user = UserId::new(random_key_id().unwrap().into_uuid()).unwrap();
    let other_user = UserId::new(random_key_id().unwrap().into_uuid()).unwrap();
    let network = NetworkId::new(random_key_id().unwrap().into_uuid()).unwrap();
    let other_network = NetworkId::new(random_key_id().unwrap().into_uuid()).unwrap();
    let list = sealer
        .seal_cursor(CursorScope::Networks(user), b"cursor payload")
        .unwrap();
    assert!(
        sealer
            .open_cursor(CursorScope::Networks(user), list.as_bytes())
            .unwrap()
            .as_bytes()
            == b"cursor payload"
    );
    assert!(
        sealer
            .open_cursor(CursorScope::Networks(other_user), list.as_bytes())
            .is_err()
    );
    assert!(
        sealer
            .open_cursor(CursorScope::Members { user, network }, list.as_bytes())
            .is_err()
    );
    let members = sealer
        .seal_cursor(CursorScope::Members { user, network }, b"members payload")
        .unwrap();
    assert!(
        sealer
            .open_cursor(
                CursorScope::Members {
                    user,
                    network: other_network
                },
                members.as_bytes()
            )
            .is_err()
    );
    assert!(
        sealer
            .open_cursor(
                CursorScope::Members {
                    user: other_user,
                    network
                },
                members.as_bytes()
            )
            .is_err()
    );
    for purpose in [Purpose::Source, Purpose::Configuration, Purpose::Bindings] {
        assert!(
            sealer
                .open(network, Revision::MIN, purpose, members.as_bytes())
                .is_err()
        );
        let config = sealer
            .seal(network, Revision::MIN, purpose, b"private configuration")
            .unwrap();
        assert!(
            sealer
                .open_cursor(CursorScope::Networks(user), config.as_bytes())
                .is_err()
        );
    }
    for i in 0..members.as_bytes().len() {
        let mut tampered = members.as_bytes().to_vec();
        tampered[i] ^= 1;
        assert!(
            sealer
                .open_cursor(CursorScope::Members { user, network }, &tampered)
                .is_err()
        );
    }
}
