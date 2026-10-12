//! Signed DNS association overlays ride the sync answer and bind its netmap.
use super::*;
use meshway_configuration::netmap::{AssociationRecordType, DnsAssociation, DnsAssociationOverlay};

fn overlay(
    netmap: &meshway_configuration::netmap::CompiledNetmap,
    name: &str,
) -> DnsAssociationOverlay {
    DnsAssociationOverlay {
        netmap: netmap.reference(),
        associations: vec![DnsAssociation {
            source: netmap.netmap.node_id,
            generation: Digest::from_bytes([1; 32]),
            name: name.into(),
            record_type: AssociationRecordType::A,
            cnames: vec![],
            addresses: vec![meshway_configuration::execution::ExecutionAddress(
                "203.0.113.9".parse().unwrap(),
            )],
            expires_at_ms: meshway_test_server::now_ms() + HOUR,
        }],
    }
}

#[tokio::test]
async fn overlays_verify_bind_the_held_netmap_and_are_named_in_the_next_request() {
    let server = Server::start().await;
    let proxy = Proxy::start(&server.origin).await;
    let (mut control, binding, script) = enrolled(&server, &proxy, "associations").await;
    let first = compiled(&binding, "associations", None);
    let mut answer = full(&binding, &first, HOUR);
    let signed_overlay = meshway_test_server::sign_associations(
        binding.network_id,
        binding.node_id,
        overlay(&first, "chat.example"),
    );
    let digest = signed_overlay.overlay.digest().unwrap();
    answer.associations = Some(signed_overlay.clone());
    script.push(Ok(answer));
    let outcome = control.sync(&mut Facts::default()).await.unwrap();
    let held = outcome
        .netmap
        .associations
        .expect("published with the netmap");
    assert_eq!(held.associations[0].name, "chat.example");
    // Unchanged (null) keeps it and the next request names its digest.
    script.push(Ok(unchanged(&binding, &first, HOUR)));
    let kept = control.sync(&mut Facts::default()).await.unwrap();
    assert!(kept.netmap.associations.is_some());
    assert_eq!(script.requests()[1].associations, Some(digest));

    // A forged signature is rejected without changing what is held.
    let mut forged = unchanged(&binding, &first, HOUR);
    let mut bad = signed_overlay.clone();
    bad.overlay.associations[0].name = "evil.example".into();
    forged.associations = Some(bad);
    script.push(Ok(forged));
    assert!(matches!(
        control.sync(&mut Facts::default()).await,
        Err(Error::SignatureInvalid)
    ));
    assert_eq!(
        control.netmap().unwrap().associations.unwrap().associations[0].name,
        "chat.example"
    );

    // New content without a new overlay drops the old one; an empty overlay clears.
    let second = compiled(&binding, "associations-two", Some(&first));
    script.push(Ok(full(&binding, &second, HOUR)));
    let next = control.sync(&mut Facts::default()).await.unwrap();
    assert!(next.netmap.associations.is_none());
    assert_eq!(script.requests().last().unwrap().associations, Some(digest));
    script.push(Ok(unchanged(&binding, &second, HOUR)));
    control.sync(&mut Facts::default()).await.unwrap();
    assert_eq!(script.requests().last().unwrap().associations, None);
    let mut cleared = unchanged(&binding, &second, HOUR);
    cleared.associations = Some(meshway_test_server::sign_associations(
        binding.network_id,
        binding.node_id,
        DnsAssociationOverlay {
            netmap: second.reference(),
            associations: vec![],
        },
    ));
    script.push(Ok(cleared));
    assert!(
        control
            .sync(&mut Facts::default())
            .await
            .unwrap()
            .netmap
            .associations
            .is_none()
    );
    control.shutdown().await.unwrap();
    server.close().await;
}
