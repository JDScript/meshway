use super::*;
use crate::state::service::ReadIdentity;
use meshway_configuration::execution::Carrier;
use meshway_model::state::ResourceRef;
use meshway_transport_spec::TransportProtocol;
use std::collections::BTreeSet;

#[tokio::test]
async fn state_visibility_is_the_netmap_projection_without_any_demand() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let lab = lab(false).await;
    let [a, b, c] = lab.nodes;
    let execution = lab.app.execution();
    let netmaps = lab.netmaps.clone();
    let tokens = lab.tokens.clone();
    let owner = crate::accounts::token_digest(&lab.owner).unwrap();
    let network = lab.network;
    lab.app
        .executor
        .handle()
        .read(move |tx| {
            let now = chrono::Utc::now();
            let node = |index: usize| ReadIdentity::Node {
                authorization: tokens[index].clone(),
            };
            let scope = |identity: &ReadIdentity| {
                execution
                    .netmap_state_scope_in_transaction(tx, &*netmaps, identity, now)
                    .unwrap()
            };
            let of_a = scope(&node(0));
            assert_eq!(of_a.visible, BTreeSet::from([a, b]));
            assert_eq!(of_a.full, BTreeSet::from([a]));
            assert!(of_a.observation_paths.is_empty());
            // Endpoint details only for the peer listener a may dial.
            assert_eq!(
                of_a.endpoint_resources,
                Some(BTreeMap::from([(
                    b,
                    BTreeSet::from([ResourceRef::Listener { key: "udp".into() }])
                )]))
            );
            assert_eq!(scope(&node(1)).visible, BTreeSet::from([a, b]));
            assert_eq!(scope(&node(2)).visible, BTreeSet::from([c]));
            assert_eq!(scope(&node(2)).endpoint_resources, Some(BTreeMap::new()));
            // The demand-derived scope of the previous chain shows nothing of
            // b without a business demand; the netmap scope does not depend on it.
            assert_eq!(
                execution
                    .state_scope_in_transaction(tx, &node(0), now)
                    .unwrap()
                    .visible,
                BTreeSet::from([a])
            );
            let manager = scope(&ReadIdentity::UserSession {
                digest: owner,
                network,
            });
            assert_eq!(manager.visible, BTreeSet::from([a, b, c]));
            assert_eq!(manager.endpoint_resources, None);
            Ok(())
        })
        .await
        .unwrap();
    lab.app.shutdown().await.unwrap();
}

#[tokio::test]
async fn udp_coordination_requires_mutual_netmaps_and_a_native_udp_carrier() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let lab = lab(false).await;
    let [a, b, c] = lab.nodes;
    let (na, nb, nc) = (
        lab.netmap(a).await,
        lab.netmap(b).await,
        lab.netmap(c).await,
    );
    let carrier = |dialer, listener, key: &str, protocol| Carrier {
        dialer,
        listener,
        listener_key: key.into(),
        protocol,
    };
    let allowed = |x: &CompiledNetmap, y: &CompiledNetmap, carrier: &Carrier| {
        crate::execution::netmap_udp_coordination(&x.netmap, &y.netmap, carrier).unwrap()
    };
    let udp = TransportProtocol::NativeUdp;
    assert!(allowed(&na, &nb, &carrier(a, b, "udp", udp)));
    assert!(allowed(&nb, &na, &carrier(a, b, "udp", udp)));
    assert!(allowed(&na, &nb, &carrier(b, a, "udp", udp)));
    assert!(!allowed(&na, &nb, &carrier(a, b, "tcp", udp)));
    assert!(!allowed(
        &na,
        &nb,
        &carrier(a, b, "udp", TransportProtocol::NativeTcp)
    ));
    assert!(!allowed(&nc, &nb, &carrier(c, b, "udp", udp)));
    assert!(!allowed(&na, &nc, &carrier(a, c, "udp", udp)));
    assert!(!allowed(&na, &na, &carrier(a, a, "udp", udp)));
    // A stale key binding in either netmap is not mutual visibility.
    let mut stale = (*nb).clone();
    stale.netmap.self_.self_keys.dataplane_key.generation = Version::new(9).unwrap();
    assert!(!allowed(&na, &stale, &carrier(a, b, "udp", udp)));
    lab.app.shutdown().await.unwrap();
}
