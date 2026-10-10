use super::*;

#[tokio::test]
async fn member_removal_atomically_deletes_user_nodes_and_preserves_tag_nodes() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, tag, _, _, _, _, _) = fixture().await;
    let owner = app
        .accounts()
        .login(
            "owner@example.com".into(),
            Secret::new("a sufficiently long password".into()),
            rid(),
        )
        .await
        .unwrap();
    let member = app
        .accounts()
        .register(
            "member@example.com".into(),
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
    let (_, membership) = app
        .networks()
        .redeem_join_link(member.token.expose(), link.secret.unwrap().expose(), rid())
        .await
        .unwrap();
    let signer = SigningKey::from_bytes(&[41; 32]);
    let enrolled = enroll_user_node(&app, member.token.expose(), network, &signer, [51; 32]).await;
    let node_session = session(
        &app.execution(),
        network,
        enrolled.node_id,
        &enrolled.keys,
        &signer,
    )
    .await;
    let lease = app
        .execution()
        .acquire(
            node_session.token.expose(),
            AcquireInstanceRequest {
                instance_id: iid(),
                capabilities: caps(),
            },
        )
        .await
        .unwrap();
    let demand = DemandRequest {
        instance: InstanceRequest {
            instance_id: lease.instance_id,
            generation: lease.generation,
        },
        request_id: rid(),
        target: BusinessTarget::Mesh { node: tag },
        protocol: BusinessProtocol::Tcp,
        port: Some(443.try_into().unwrap()),
        profile: "mesh".into(),
        duration_seconds: 30,
    };
    app.execution()
        .demand(node_session.token.expose(), demand.clone())
        .await
        .unwrap();
    let impact = app
        .networks()
        .removal_impact(member.token.expose(), network, None, rid())
        .await
        .unwrap();
    assert_eq!(impact.node_ids, vec![enrolled.node_id]);
    let event_sequence = app.state_runtime.lock().unwrap().hub.events.sequence();
    app.executor.handle().execute(|db| db.execute_batch("CREATE TRIGGER fail_member_removal BEFORE UPDATE OF revoked_at ON memberships BEGIN SELECT RAISE(ABORT,'injected failure'); END;")).await.unwrap().unwrap();
    assert!(
        app.networks()
            .remove_member(
                member.token.expose(),
                network,
                None,
                impact.version,
                impact.revision,
                rid()
            )
            .await
            .is_err()
    );
    assert_eq!(
        app.networks()
            .get(owner.token.expose(), network)
            .await
            .unwrap()
            .current_revision,
        impact.revision
    );
    assert!(
        app.state_runtime
            .lock()
            .unwrap()
            .hub
            .events
            .after(event_sequence, network)
            .unwrap()
            .is_empty()
    );
    app.execution()
        .authenticate(node_session.token.expose())
        .await
        .unwrap();
    app.executor
        .handle()
        .execute(|db| db.execute_batch("DROP TRIGGER fail_member_removal;"))
        .await
        .unwrap()
        .unwrap();
    let revision = app
        .networks()
        .remove_member(
            member.token.expose(),
            network,
            Some(membership),
            impact.version,
            impact.revision,
            rid(),
        )
        .await
        .unwrap();
    assert!(revision > impact.revision);
    let changes = app
        .state_runtime
        .lock()
        .unwrap()
        .hub
        .events
        .after(event_sequence, network)
        .unwrap();
    assert_eq!(changes.iter().filter(|entry| matches!(entry.change, crate::effect::Effect::NodeRetired { node_id, .. } if node_id == enrolled.node_id)).count(),1);
    assert!(!changes.iter().any(|entry| matches!(entry.change, crate::effect::Effect::NodeRetired { node_id, .. } if node_id == tag)));
    assert!(matches!(
        app.execution()
            .authenticate(node_session.token.expose())
            .await,
        Err(Error::InvalidCredentials)
    ));
    assert_eq!(
        app.execution()
            .renew_demand(
                node_session.token.expose(),
                RenewDemandRequest {
                    instance: demand.instance,
                    request_id: demand.request_id,
                    expires_at_ms: lease.expires_at_ms,
                }
            )
            .await,
        Err(Error::InvalidCredentials)
    );
    let inventory = app
        .execution()
        .inventory(owner.token.expose(), network)
        .await
        .unwrap();
    assert!(!inventory.items.iter().any(|n| n.id == enrolled.node_id));
    assert!(inventory.items.iter().any(|n| n.id == tag));
    assert!(
        app.networks()
            .get(member.token.expose(), network)
            .await
            .is_err()
    );
    let snapshot = app
        .configurations()
        .read(owner.token.expose(), network, None)
        .await
        .unwrap();
    assert!(
        !snapshot
            .document
            .configuration
            .nodes
            .contains_key(&enrolled.node_id)
    );
    app.check_storage().await.unwrap();
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn rejoined_user_enrollment_preserves_inactive_group_membership() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, target, _, target_keys, _, target_signer, _) = fixture().await;
    let owner = app
        .accounts()
        .login(
            "owner@example.com".into(),
            Secret::new("a sufficiently long password".into()),
            rid(),
        )
        .await
        .unwrap();
    let user = app
        .accounts()
        .register(
            "rejoining@example.test".into(),
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
    let (_, old_membership) = app
        .networks()
        .redeem_join_link(user.token.expose(), link.secret.unwrap().expose(), rid())
        .await
        .unwrap();
    let old_signer = SigningKey::from_bytes(&[81; 32]);
    let old_node =
        enroll_user_node(&app, user.token.expose(), network, &old_signer, [82; 32]).await;
    let old_session = session(
        &app.execution(),
        network,
        old_node.node_id,
        &old_node.keys,
        &old_signer,
    )
    .await;
    let snapshot = app
        .configurations()
        .read(owner.token.expose(), network, None)
        .await
        .unwrap();
    app.configurations()
        .apply(
            owner.token.expose(),
            network,
            meshway_protocol::configuration::ConfigurationRequest::Operations {
                expected_revision: snapshot.revision,
                request_id: rid(),
                ops: vec![
                    ConfigurationOperation::ReplaceGroups {
                        groups: BTreeMap::from([(
                            "group:former-members".into(),
                            vec![user.session.user.email.clone()],
                        )]),
                    },
                    ConfigurationOperation::ReplaceTagsOwners {
                        tags_owners: BTreeMap::from([
                            ("tag:test".into(), vec![]),
                            (
                                "tag:former-owned".into(),
                                vec![user.session.user.email.clone()],
                            ),
                        ]),
                    },
                    ConfigurationOperation::ReplaceAccess {
                        access: vec![AccessRule {
                            remark: String::new(),
                            src: vec!["group:former-members".into()],
                            dst: vec![format!("node:{target}")],
                            ip: vec!["tcp:443".into()],
                            bidirectional: false,
                        }],
                    },
                ],
            },
            rid(),
        )
        .await
        .unwrap();
    let impact = app
        .networks()
        .removal_impact(owner.token.expose(), network, Some(old_membership), rid())
        .await
        .unwrap();
    assert_eq!(impact.node_ids, vec![old_node.node_id]);
    app.networks()
        .remove_member(
            owner.token.expose(),
            network,
            Some(old_membership),
            impact.version,
            impact.revision,
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
    let (_, new_membership) = app
        .networks()
        .redeem_join_link(user.token.expose(), link.secret.unwrap().expose(), rid())
        .await
        .unwrap();
    assert_ne!(old_membership, new_membership);
    let before = app
        .configurations()
        .read(owner.token.expose(), network, None)
        .await
        .unwrap();
    assert_eq!(
        before.bindings.users[&user.session.user.email].membership_id,
        old_membership
    );
    assert!(
        !before
            .document
            .configuration
            .nodes
            .contains_key(&old_node.node_id)
    );
    let signer = SigningKey::from_bytes(&[83; 32]);
    let result = try_enroll_user_node(&app, user.token.expose(), network, &signer, [84; 32]).await;
    if result.is_err() {
        let unchanged = app
            .configurations()
            .read(owner.token.expose(), network, None)
            .await
            .unwrap();
        assert_eq!(unchanged.revision, before.revision);
        assert_eq!(unchanged.bindings.users, before.bindings.users);
        assert_eq!(
            unchanged.document.configuration,
            before.document.configuration
        );
        app.check_storage().await.unwrap();
    }
    // System enrollment must neither reject the current member due to an old
    // group reference nor reactivate that group's old membership binding.
    let enrolled =
        result.expect("current membership must enroll without rebinding the inactive group");
    let after = app
        .configurations()
        .read(owner.token.expose(), network, None)
        .await
        .unwrap();
    assert_eq!(
        after.document.configuration.groups,
        before.document.configuration.groups
    );
    assert_eq!(after.bindings.users, before.bindings.users);
    assert_eq!(
        after.bindings.node_users[&enrolled.node_id],
        UserBinding {
            user_id: user.session.user.id,
            membership_id: new_membership
        }
    );
    assert!(!after.bindings.node_users.contains_key(&old_node.node_id));
    assert!(!after.bindings.node_users.contains_key(&target));
    let comparison = app
        .configurations()
        .compare(
            owner.token.expose(),
            network,
            meshway_protocol::configuration::CompareConfigurationRequest {
                from_revision: before.revision,
                to_revision: after.revision,
            },
            rid(),
        )
        .await
        .unwrap();
    assert!(
        comparison
            .changes
            .iter()
            .any(|change| change.path == "/bindings/nodeUsers")
    );
    assert!(
        !comparison
            .changes
            .iter()
            .any(|change| change.path == "/bindings/users")
    );
    let history = app
        .configurations()
        .read(owner.token.expose(), network, Some(snapshot.revision))
        .await
        .unwrap();
    assert_eq!(
        history.bindings.node_users[&old_node.node_id].membership_id,
        old_membership
    );
    assert!(
        app.configurations()
            .apply(
                owner.token.expose(),
                network,
                meshway_protocol::configuration::ConfigurationRequest::Restore {
                    expected_revision: after.revision,
                    rule_set_bindings: None,
                    request_id: rid(),
                    restore_from: snapshot.revision
                },
                rid()
            )
            .await
            .is_err()
    );
    let service = app.execution();
    assert_eq!(
        service
            .challenge(service.signed_challenge_fixture(
                NodeChallengeRequest {
                    network_id: network,
                    node_id: old_node.node_id,
                    key_id: old_node.keys.control.id,
                    operation: ProofOperation::Session,
                    request_digest:
                        proof::request_digest(&NodeSessionIntent { request_id: rid() }).unwrap()
                },
                &old_signer
            ))
            .await
            .err(),
        Some(Error::InvalidCredentials)
    );
    let node_session = session(&service, network, enrolled.node_id, &enrolled.keys, &signer).await;
    let target_name = node_dns_name(
        &after.document.configuration.nodes[&target].hostname,
        &after.document.configuration.dns.domain,
    )
    .unwrap();
    assert!(matches!(
        service
            .query_dns(
                old_session.token.expose(),
                NodeDnsQueryRequest {
                    name: target_name.clone()
                },
                rid()
            )
            .await,
        Err(Error::InvalidCredentials)
    ));
    // Rejoining does not reactivate the former membership's group grant.
    assert!(
        service
            .query_dns(
                node_session.token.expose(),
                NodeDnsQueryRequest {
                    name: target_name.clone()
                },
                rid()
            )
            .await
            .unwrap()
            .record
            .is_none()
    );
    let lease = service
        .acquire(
            node_session.token.expose(),
            AcquireInstanceRequest {
                instance_id: iid(),
                capabilities: caps(),
            },
        )
        .await
        .unwrap();
    let decision = service
        .demand(
            node_session.token.expose(),
            DemandRequest {
                instance: InstanceRequest {
                    instance_id: lease.instance_id,
                    generation: lease.generation,
                },
                request_id: rid(),
                target: BusinessTarget::Mesh { node: target },
                protocol: BusinessProtocol::Tcp,
                port: Some(443.try_into().unwrap()),
                profile: "mesh".into(),
                duration_seconds: 60,
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        decision.decision,
        DemandDecision::Unavailable {
            reason: UnavailableReason::Unauthorized
        }
    ));
    // An ordinary source save keeps both purposes; it cannot rebind the policy
    // slot to M2 merely because a new Node now has the same readable email.
    let saved = app
        .configurations()
        .apply(
            owner.token.expose(),
            network,
            meshway_protocol::configuration::ConfigurationRequest::Document {
                expected_revision: after.revision,
                request_id: rid(),
                source_jsonc: WireSecret::new(
                    canonical_json(&after.document.configuration)
                        .unwrap()
                        .expose()
                        .into(),
                ),
            },
            rid(),
        )
        .await
        .unwrap();
    let preview = app
        .configurations()
        .validate(
            owner.token.expose(),
            network,
            meshway_protocol::configuration::ConfigurationRequest::Operations {
                expected_revision: saved.revision,
                request_id: rid(),
                ops: vec![ConfigurationOperation::SetNodeSettings {
                    node_id: enrolled.node_id,
                    fields: NodeSettingsPatch {
                        remark: Input::Value("new membership device".into()),
                        ..Default::default()
                    },
                }],
            },
            rid(),
        )
        .await
        .unwrap();
    assert!(
        preview
            .unavailable_dependencies
            .iter()
            .any(|issue| issue.path.starts_with("/tagsOwners/"))
    );
    let target_session = session(&service, network, target, &target_keys, &target_signer).await;
    service
        .acquire(
            target_session.token.expose(),
            AcquireInstanceRequest {
                instance_id: iid(),
                capabilities: caps(),
            },
        )
        .await
        .unwrap();
    let authorized = app
        .configurations()
        .apply(
            owner.token.expose(),
            network,
            meshway_protocol::configuration::ConfigurationRequest::Operations {
                expected_revision: saved.revision,
                request_id: rid(),
                ops: vec![ConfigurationOperation::ReplaceAccess {
                    access: vec![AccessRule {
                        remark: String::new(),
                        src: vec![format!("node:{}", enrolled.node_id)],
                        dst: vec![format!("node:{target}")],
                        ip: vec!["tcp:443".into()],
                        bidirectional: false,
                    }],
                }],
            },
            rid(),
        )
        .await
        .unwrap();
    let permitted = service
        .demand(
            node_session.token.expose(),
            DemandRequest {
                instance: InstanceRequest {
                    instance_id: lease.instance_id,
                    generation: lease.generation,
                },
                request_id: rid(),
                target: BusinessTarget::Mesh { node: target },
                protocol: BusinessProtocol::Tcp,
                port: Some(443.try_into().unwrap()),
                profile: "mesh".into(),
                duration_seconds: 60,
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        permitted.decision,
        DemandDecision::Permitted { .. }
    ));
    let preserved = app
        .configurations()
        .read(owner.token.expose(), network, None)
        .await
        .unwrap();
    assert_eq!(preserved.bindings.users, before.bindings.users);
    assert_eq!(
        preserved.bindings.node_users[&enrolled.node_id].membership_id,
        new_membership
    );
    let impact = app
        .networks()
        .removal_impact(owner.token.expose(), network, Some(new_membership), rid())
        .await
        .unwrap();
    assert_eq!(impact.revision, authorized.revision);
    assert_eq!(impact.node_ids, vec![enrolled.node_id]);
    assert_eq!(
        service
            .query_dns(
                node_session.token.expose(),
                NodeDnsQueryRequest {
                    name: target_name.clone()
                },
                rid()
            )
            .await
            .unwrap()
            .record
            .unwrap()
            .node_id,
        target
    );
    app.networks()
        .remove_member(
            owner.token.expose(),
            network,
            Some(new_membership),
            impact.version,
            impact.revision,
            rid(),
        )
        .await
        .unwrap();
    assert_eq!(
        service
            .authenticate(node_session.token.expose())
            .await
            .err(),
        Some(Error::InvalidCredentials)
    );
    let removed = app
        .configurations()
        .read(owner.token.expose(), network, None)
        .await
        .unwrap();
    assert!(!removed.bindings.node_users.contains_key(&enrolled.node_id));
    assert_eq!(removed.bindings.users, before.bindings.users);
    assert!(matches!(
        service
            .query_dns(
                node_session.token.expose(),
                NodeDnsQueryRequest { name: target_name },
                rid()
            )
            .await,
        Err(Error::InvalidCredentials)
    ));
    app.check_storage().await.unwrap();
    app.shutdown().await.unwrap();
}
