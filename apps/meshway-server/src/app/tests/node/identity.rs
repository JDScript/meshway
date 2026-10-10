use super::*;

#[tokio::test]
async fn email_maintenance_preserves_live_node_subject_and_session() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, _, _, _, _, _, _) = fixture().await;
    let owner = app
        .accounts()
        .login(
            "owner@example.com".into(),
            Secret::new("a sufficiently long password".into()),
            rid(),
        )
        .await
        .unwrap();
    let signer = SigningKey::from_bytes(&[85; 32]);
    let peer = app
        .accounts()
        .register(
            "pupil@example.com".into(),
            Secret::new("a sufficiently long password".into()),
            rid(),
        )
        .await
        .unwrap();
    let link = app
        .networks()
        .create_join_link(
            owner.token.expose(),
            network,
            MembershipRole::Member,
            600,
            rid(),
        )
        .await
        .unwrap();
    app.networks()
        .redeem_join_link(peer.token.expose(), link.secret.unwrap().expose(), rid())
        .await
        .unwrap();
    let node = enroll_user_node(&app, owner.token.expose(), network, &signer, [86; 32]).await;
    let service = app.execution();
    let session = session(&service, network, node.node_id, &node.keys, &signer).await;
    let lease = service
        .acquire(
            session.token.expose(),
            AcquireInstanceRequest {
                instance_id: iid(),
                capabilities: caps(),
            },
        )
        .await
        .unwrap();
    let before = app
        .configurations()
        .read(owner.token.expose(), network, None)
        .await
        .unwrap();
    app.configurations()
        .apply(
            owner.token.expose(),
            network,
            meshway_protocol::configuration::ConfigurationRequest::Operations {
                expected_revision: before.revision,
                request_id: rid(),
                ops: vec![ConfigurationOperation::ReplaceGroups {
                    groups: BTreeMap::from([(
                        "group:team".into(),
                        vec!["owner@example.com".into(), "pupil@example.com".into()],
                    )]),
                }],
            },
            rid(),
        )
        .await
        .unwrap();
    let changed = app
        .accounts()
        .change_email(
            owner.token.expose(),
            Secret::new("a sufficiently long password".into()),
            "renamed-owner@example.com".into(),
            rid(),
            app.configurations().committer(),
        )
        .await
        .unwrap();
    assert_eq!(changed.id, owner.session.user.id);
    assert_eq!(
        service
            .authenticate(session.token.expose())
            .await
            .unwrap()
            .node_id(),
        node.node_id
    );
    let next = service
        .renew(
            session.token.expose(),
            InstanceRequest {
                instance_id: lease.instance_id,
                generation: lease.generation,
            },
        )
        .await
        .unwrap();
    assert_eq!(next.instance_id, lease.instance_id);
    let after = app
        .configurations()
        .read(owner.token.expose(), network, None)
        .await
        .unwrap();
    assert_eq!(
        before.bindings.node_users[&node.node_id],
        after.bindings.node_users[&node.node_id]
    );
    assert_eq!(
        after.document.configuration.nodes[&node.node_id].identity,
        NodeIdentity::User {
            user: "renamed-owner@example.com".into()
        }
    );
    app.shutdown().await.unwrap();
}
