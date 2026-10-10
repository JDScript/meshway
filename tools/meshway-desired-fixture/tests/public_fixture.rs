use meshway_desired_fixture::{
    NodeMaterial,
    public_fixture::{PublicFixture, PublicNode},
    random_uuid,
};
use meshway_model::{
    KeyId, NodeId,
    state::{Capability, StreamKind, TransportRole},
};
use std::collections::BTreeMap;

#[test]
fn independent_compilers_match_and_pair_registered_keys() {
    for count in [2, 3] {
        let mut secrets = Vec::new();
        let mut nodes = Vec::new();
        for i in 0..count {
            let id = NodeId::new(random_uuid().unwrap()).unwrap();
            let signing = KeyId::new(random_uuid().unwrap()).unwrap();
            let noise = KeyId::new(random_uuid().unwrap()).unwrap();
            let capabilities = BTreeMap::from([(
                "transport:native_udp".into(),
                Capability::Transport {
                    roles: vec![TransportRole::Dial, TransportRole::Listen],
                    streams: vec![StreamKind::Udp],
                },
            )]);
            let material = NodeMaterial::from_seeds(
                [i + 1; 32],
                [i + 11; 32],
                signing,
                noise,
                capabilities.clone(),
            );
            secrets.push((id, i, signing, noise, capabilities));
            nodes.push(PublicNode {
                id,
                socket: format!("127.0.0.{}:41000", i + 1).parse().unwrap(),
                trusted: material.trusted().clone(),
            });
        }
        let fixture = PublicFixture::iperf(nodes).unwrap();
        let bytes = serde_json::to_vec(&fixture).unwrap();
        for (id, i, signing, noise, capabilities) in secrets {
            let decoded: PublicFixture = serde_json::from_slice(&bytes).unwrap();
            let mut tampered = decoded.clone();
            tampered.netmaps.values_mut().next().unwrap().serial =
                meshway_model::Version::new(9).unwrap();
            assert!(
                tampered
                    .issue(
                        id,
                        NodeMaterial::from_seeds(
                            [i + 1; 32],
                            [i + 11; 32],
                            signing,
                            noise,
                            capabilities.clone()
                        )
                    )
                    .is_err()
            );
            assert!(
                decoded
                    .issue(
                        id,
                        NodeMaterial::from_seeds(
                            [i + 2; 32],
                            [i + 11; 32],
                            signing,
                            noise,
                            capabilities.clone()
                        )
                    )
                    .is_err()
            );
            let material =
                NodeMaterial::from_seeds([i + 1; 32], [i + 11; 32], signing, noise, capabilities);
            let authority = decoded.issue(id, material).unwrap();
            assert_eq!(
                authority.netmap(id).unwrap().reference(),
                fixture.netmaps[&id]
            );
            let desired = authority.desired(id).unwrap();
            assert!(
                authority
                    .keys(id)
                    .unwrap()
                    .matches(&desired.local().self_keys)
            );
        }
        let mut changed: PublicFixture = serde_json::from_slice(&bytes).unwrap();
        changed.configuration.access[0].ip = vec!["tcp:5202".into()];
        let changed = changed.compile().unwrap();
        assert!(
            fixture
                .netmaps
                .iter()
                .any(|(id, reference)| changed[id].reference().digest != reference.digest)
        );
    }
}
