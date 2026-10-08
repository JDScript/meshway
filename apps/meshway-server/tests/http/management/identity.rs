use super::*;

#[tokio::test]
async fn removed_member_references_survive_email_maintenance_and_never_rebind_on_save() {
    let _serial = ACCOUNT_FLOW.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let s = Server::start(&dir.path().canonicalize().unwrap().join("controller"), true).await;
    let c = client();
    let owner = register(&c, &s, "reference-owner@example.test", "valid password").await;
    let member = register(&c, &s, "departing@example.test", "valid password").await;
    let n = create(&c, &s, owner.token.expose(), "foreign network")
        .await
        .network
        .id;
    let old_member = join(
        &c,
        &s,
        n,
        owner.token.expose(),
        member.token.expose(),
        MembershipRole::Member,
    )
    .await;
    let initial = source(&c, &s, n, owner.token.expose()).await;
    let mut input = meshway_configuration::NetworkConfigurationInput::from(&initial.configuration);
    input
        .groups
        .insert("group:team".into(), vec![member.user.email.clone()]);
    input
        .tags_owners
        .insert("tag:backend".into(), vec![member.user.email.clone()]);
    let applied: ConfigurationAppliedResponse = json(
        c.post(net(&s, n, "configuration"))
            .bearer_auth(owner.token.expose())
            .json(&ConfigurationRequest::Document {
                expected_revision: initial.revision,
                request_id: request(),
                source_jsonc: WireSecret::new(serde_json::to_string(&input).unwrap()),
            }),
    )
    .await;
    c.post(net(&s, n, &format!("members/{old_member}/remove")))
        .bearer_auth(owner.token.expose())
        .json(&RemoveMemberRequest {
            expected_version: meshway_model::Version::MIN,
            expected_revision: applied.revision,
        })
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let changed: AccountResponse = json(
        c.post(format!("{}/api/v1/account/email", s.url))
            .bearer_auth(member.token.expose())
            .json(&ChangeEmailRequest {
                password: WireSecret::new("valid password".into()),
                email: "departed-new@example.test".into(),
            }),
    )
    .await;
    assert_eq!(changed.user.id, member.user.id);
    // The revoked membership's references keep the departed name; the former
    // network never learns the new address and records no maintenance version.
    let after_departure = source(&c, &s, n, owner.token.expose()).await;
    assert_eq!(after_departure.revision, applied.revision);
    assert!(
        after_departure
            .source_jsonc
            .expose()
            .contains("departing@example.test")
    );
    assert!(
        !after_departure
            .source_jsonc
            .expose()
            .contains("departed-new@example.test")
    );
    let new_member = join(
        &c,
        &s,
        n,
        owner.token.expose(),
        member.token.expose(),
        MembershipRole::Member,
    )
    .await;
    assert_ne!(old_member, new_member);
    let replacement = register(&c, &s, "departing@example.test", "valid password").await;
    join(
        &c,
        &s,
        n,
        owner.token.expose(),
        replacement.token.expose(),
        MembershipRole::Member,
    )
    .await;
    let current = source(&c, &s, n, owner.token.expose()).await;
    for edit in [
        ConfigurationRequest::Document {
            expected_revision: current.revision,
            request_id: request(),
            source_jsonc: WireSecret::new(current.source_jsonc.expose().into()),
        },
        ConfigurationRequest::Operations {
            expected_revision: current.revision,
            request_id: request(),
            ops: vec![],
        },
    ] {
        let preview: ConfigurationPreviewResponse = json(
            c.post(net(&s, n, "configuration/validate"))
                .bearer_auth(owner.token.expose())
                .json(&edit),
        )
        .await;
        assert_eq!(preview.change_count, 0);
        assert!(preview.changes.is_empty());
        assert!(!preview.source_changed);
        assert!(!preview.unavailable_dependencies.is_empty());
        let result: ConfigurationAppliedResponse = json(
            c.post(net(&s, n, "configuration"))
                .bearer_auth(owner.token.expose())
                .json(&edit),
        )
        .await;
        assert_eq!(result.revision, current.revision);
    }
    let restore = c
        .post(net(&s, n, "configuration"))
        .bearer_auth(owner.token.expose())
        .json(&ConfigurationRequest::Restore {
            expected_revision: current.revision,
            resource_publication: None,
            request_id: request(),
            restore_from: applied.revision,
        })
        .send()
        .await
        .unwrap();
    assert_eq!(restore.status(), StatusCode::SERVICE_UNAVAILABLE);
    let unknown = ConfigurationRequest::Operations {
        expected_revision: current.revision,
        request_id: request(),
        ops: vec![ConfigurationOperation::ReplaceGroups {
            groups: [("group:unknown".into(), vec!["unknown@example.test".into()])].into(),
        }],
    };
    assert_eq!(
        c.post(net(&s, n, "configuration/validate"))
            .bearer_auth(owner.token.expose())
            .json(&unknown)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let activity: ActivityListResponse = json(
        c.get(net(&s, n, "activity"))
            .bearer_auth(owner.token.expose()),
    )
    .await;
    assert!(activity.items.iter().any(|event| matches!(event.target, ActivityTarget::Membership { membership_id } if membership_id == old_member)));
    s.close().await;
}

#[tokio::test]
async fn management_activities_identify_transactional_member_and_link_targets() {
    let _serial = ACCOUNT_FLOW.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let s = Server::start(&dir.path().canonicalize().unwrap().join("controller"), true).await;
    let c = client();
    let owner = register(&c, &s, "activity-owner@example.test", "valid password").await;
    let member = register(&c, &s, "activity-member@example.test", "valid password").await;
    let n = create(&c, &s, owner.token.expose(), "activity")
        .await
        .network
        .id;
    let member_id = join(
        &c,
        &s,
        n,
        owner.token.expose(),
        member.token.expose(),
        MembershipRole::Member,
    )
    .await;
    let promoted: MemberMutationResponse = json(
        c.post(net(&s, n, &format!("members/{member_id}/role")))
            .bearer_auth(owner.token.expose())
            .json(&ChangeMemberRoleRequest {
                expected_version: meshway_model::Version::MIN,
                role: MembershipRole::Admin,
            }),
    )
    .await;
    let link: JoinLinkCreated = json(
        c.post(net(&s, n, "join-links"))
            .bearer_auth(owner.token.expose())
            .json(&CreateJoinLinkRequest {
                request_id: request(),
                role: MembershipRole::Member,
                expires_in_seconds: 600,
            }),
    )
    .await;
    let JoinLinkCreated::Available { link, .. } = link else {
        panic!("first delivery")
    };
    c.post(net(&s, n, &format!("join-links/{}/revoke", link.id)))
        .bearer_auth(owner.token.expose())
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    c.post(net(&s, n, &format!("members/{member_id}/remove")))
        .bearer_auth(owner.token.expose())
        .json(&RemoveMemberRequest {
            expected_version: promoted.member.version,
            expected_revision: Revision::MIN,
        })
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let events: ActivityListResponse = json(
        c.get(net(&s, n, "activity"))
            .bearer_auth(owner.token.expose()),
    )
    .await;
    for action in [
        ActivityAction::MembershipJoin,
        ActivityAction::MembershipRole,
        ActivityAction::MembershipRemove,
    ] {
        let event = events
            .items
            .iter()
            .find(|event| event.action == action)
            .unwrap();
        assert!(
            matches!(event.target, ActivityTarget::Membership { membership_id } if membership_id == member_id)
        );
    }
    for action in [
        ActivityAction::JoinLinkCreate,
        ActivityAction::JoinLinkRevoke,
    ] {
        assert!(events.items.iter().any(|event| event.action == action
            && matches!(event.target, ActivityTarget::JoinLink { link_id } if link_id == link.id)));
    }
    s.close().await;
}

#[tokio::test]
async fn email_sync_preserves_subject_history_comments_and_restore_binding() {
    let _serial = ACCOUNT_FLOW.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let s = Server::start(&dir.path().canonicalize().unwrap().join("controller"), true).await;
    let c = client();
    let owner = register(&c, &s, "before@example.test", "valid password").await;
    let network = create(&c, &s, owner.token.expose(), "references")
        .await
        .network;
    let source: ConfigurationSourceResponse = json(
        c.get(net(&s, network.id, "configuration/source"))
            .bearer_auth(owner.token.expose()),
    )
    .await;
    let mut input = meshway_configuration::NetworkConfigurationInput::from(&source.configuration);
    input
        .groups
        .insert("group:team".into(), vec!["before@example.test".into()]);
    let edit = ConfigurationRequest::Document {
        expected_revision: Revision::MIN,
        request_id: request(),
        source_jsonc: WireSecret::new(format!(
            "// before@example.test is a comment\n{}",
            serde_json::to_string(&input).unwrap()
        )),
    };
    let response = c
        .post(net(&s, network.id, "configuration"))
        .bearer_auth(owner.token.expose())
        .json(&edit)
        .send()
        .await
        .unwrap();
    if response.status() != StatusCode::OK {
        panic!("{}", response.text().await.unwrap());
    }
    let applied: ConfigurationAppliedResponse = response.json().await.unwrap();
    let changed: AccountResponse = json(
        c.post(format!("{}/api/v1/account/email", s.url))
            .bearer_auth(owner.token.expose())
            .json(&ChangeEmailRequest {
                password: WireSecret::new("valid password".into()),
                email: "after@example.test".into(),
            }),
    )
    .await;
    assert_eq!(changed.user.id, owner.user.id);
    let source: ConfigurationSourceResponse = json(
        c.get(net(&s, network.id, "configuration/source"))
            .bearer_auth(owner.token.expose()),
    )
    .await;
    assert_eq!(source.revision.get(), applied.revision.get() + 1);
    assert_eq!(
        source.configuration.groups["group:team"],
        vec!["after@example.test"]
    );
    assert!(
        source
            .source_jsonc
            .expose()
            .starts_with("// before@example.test is a comment")
    );
    // Reusing the freed name must not retarget an immutable version to its new owner.
    let replacement = register(&c, &s, "before@example.test", "valid password").await;
    assert_ne!(replacement.user.id, owner.user.id);
    join(
        &c,
        &s,
        network.id,
        owner.token.expose(),
        replacement.token.expose(),
        MembershipRole::Member,
    )
    .await;
    let restored: ConfigurationAppliedResponse = json(
        c.post(net(&s, network.id, "configuration"))
            .bearer_auth(owner.token.expose())
            .json(&ConfigurationRequest::Restore {
                expected_revision: source.revision,
                resource_publication: None,
                request_id: request(),
                restore_from: applied.revision,
            }),
    )
    .await;
    let source: ConfigurationSourceResponse = json(
        c.get(net(&s, network.id, "configuration/source"))
            .bearer_auth(owner.token.expose()),
    )
    .await;
    assert_eq!(source.revision, restored.revision);
    assert_eq!(
        source.configuration.groups["group:team"],
        vec!["after@example.test"]
    );
    let original: ConfigurationSourceResponse = json(
        c.get(net(
            &s,
            network.id,
            &format!("configuration/source?revision={}", applied.revision.get()),
        ))
        .bearer_auth(owner.token.expose()),
    )
    .await;
    assert_eq!(
        original.configuration.groups["group:team"],
        vec!["before@example.test"]
    );
    s.close().await;
}
