use super::*;
use ed25519_dalek::{Signature, VerifyingKey};
use meshway_model::{KeyAlgorithm, NodePublicKey, Version};

#[test]
fn packet_proofs_bind_domain_network_node_key_generation_and_digest() {
    let secret = SigningKey::from_bytes(&[7; 32]);
    let network: NetworkId = "00000000-0000-4000-8000-000000000001".parse().unwrap();
    let node: NodeId = "00000000-0000-4000-8000-000000000002".parse().unwrap();
    let key = BoundNodeKey {
        id: "00000000-0000-4000-8000-000000000003".parse().unwrap(),
        generation: Version::MIN,
        public_key: NodePublicKey {
            algorithm: KeyAlgorithm::Ed25519,
            bytes: secret.verifying_key().to_bytes().into(),
        },
    };
    let digest: Digest = [9; 32].into();
    let message = packet_proof_message(network, node, &key, digest);
    assert!(message.starts_with(b"meshway/packet-proof/v2\0"));
    assert_eq!(message.len(), 112);
    let signature: Signature = secret.sign(&message);
    let verifier: VerifyingKey = secret.verifying_key();
    verifier.verify_strict(&message, &signature).unwrap();
    let mut v1 = message.clone();
    v1[b"meshway/packet-proof/v".len()] = b'1';
    assert!(verifier.verify_strict(&v1, &signature).is_err());
    let mut changed_key = key.clone();
    changed_key.generation = Version::new(2).unwrap();
    for altered in [
        packet_proof_message(
            "00000000-0000-4000-8000-000000000004".parse().unwrap(),
            node,
            &key,
            digest,
        ),
        packet_proof_message(
            network,
            "00000000-0000-4000-8000-000000000004".parse().unwrap(),
            &key,
            digest,
        ),
        packet_proof_message(network, node, &changed_key, digest),
        packet_proof_message(network, node, &key, [10; 32].into()),
        digest.as_bytes().to_vec(),
    ] {
        assert!(verifier.verify_strict(&altered, &signature).is_err());
    }
    changed_key = key;
    changed_key.id = "00000000-0000-4000-8000-000000000004".parse().unwrap();
    assert!(
        verifier
            .verify_strict(
                &packet_proof_message(network, node, &changed_key, digest),
                &signature
            )
            .is_err()
    );
}
