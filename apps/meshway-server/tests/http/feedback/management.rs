use super::*;

#[tokio::test]
async fn join_link_actions_pagination_inspection_and_issuer_permission_loss() {
    let _serial = ACCOUNT_FLOW.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let s = Server::start(&dir.path().canonicalize().unwrap().join("controller"), true).await;
    let c = client();
    let owner = register(&c, &s, "links-owner@example.test", "valid password").await;
    let admin = register(&c, &s, "links-admin@example.test", "valid password").await;
    let member = register(&c, &s, "links-member@example.test", "valid password").await;
    let n = create(&c, &s, owner.token.expose(), "links")
        .await
        .network
        .id;
    let admin_id = join(
        &c,
        &s,
        n,
        owner.token.expose(),
        admin.token.expose(),
        MembershipRole::Admin,
    )
    .await;
    for (user, expected) in [
        (
            &owner,
            vec![
                MembershipRole::Owner,
                MembershipRole::Admin,
                MembershipRole::Member,
            ],
        ),
        (&admin, vec![MembershipRole::Member]),
    ] {
        let network: meshway_protocol::NetworkResponse = value(
            c.get(net(&s, n, "").trim_end_matches('/'))
                .bearer_auth(user.token.expose()),
        )
        .await;
        assert_eq!(network.network.actions.join_link_roles, expected);
    }
    let make = |token: &str, role| {
        c.post(net(&s, n, "join-links"))
            .bearer_auth(token)
            .json(&CreateJoinLinkRequest {
                request_id: request(),
                role,
                expires_in_seconds: 600,
            })
    };
    error(
        make(admin.token.expose(), MembershipRole::Owner)
            .send()
            .await
            .unwrap(),
        StatusCode::FORBIDDEN,
        "permission_denied",
    )
    .await;
    let owner_link: JoinLinkCreated =
        value(make(owner.token.expose(), MembershipRole::Owner)).await;
    let JoinLinkCreated::Available {
        link: owner_link,
        secret: owner_secret,
        ..
    } = owner_link
    else {
        panic!()
    };
    let admin_link: JoinLinkCreated =
        value(make(admin.token.expose(), MembershipRole::Member)).await;
    let JoinLinkCreated::Available {
        link: admin_link,
        secret: admin_secret,
        ..
    } = admin_link
    else {
        panic!()
    };
    assert!(owner_link.actions.revoke && admin_link.actions.revoke);
    assert_eq!(owner_link.issued_by.user_id, owner.user.id);
    assert_eq!(admin_link.issued_by.user_id, admin.user.id);
    assert_eq!(admin_link.issued_by.email, "links-admin@example.test");
    assert_eq!(admin_link.consumed_by, None);
    let inspect = |token: &str, secret: &WireSecret| {
        c.post(format!("{}/api/v1/join-links/inspect", s.url))
            .bearer_auth(token)
            .json(&JoinLinkCredentialRequest {
                token: WireSecret::new(secret.expose().into()),
            })
    };
    let inspection: JoinLinkInspection = value(inspect(owner.token.expose(), &owner_secret)).await;
    assert!(inspection.already_member);
    let first: JoinLinkListResponse = value(
        c.get(net(&s, n, "join-links?limit=1"))
            .bearer_auth(admin.token.expose()),
    )
    .await;
    assert_eq!(first.total.get(), 3);
    assert_eq!(first.items[0].id, admin_link.id);
    let cursor = first.next_cursor.unwrap();
    let _: JoinLinkCreated = value(make(owner.token.expose(), MembershipRole::Member)).await;
    let second: JoinLinkListResponse = value(
        c.get(net(&s, n, &format!("join-links?limit=1&cursor={cursor}")))
            .bearer_auth(admin.token.expose()),
    )
    .await;
    assert_eq!(second.total.get(), 3);
    assert_eq!(second.items[0].id, owner_link.id);
    assert!(!second.items[0].actions.revoke);
    reject(
        c.post(net(&s, n, &format!("join-links/{}/revoke", owner_link.id)))
            .bearer_auth(admin.token.expose()),
        StatusCode::FORBIDDEN,
        "permission_denied",
    )
    .await;
    let _: MemberMutationResponse = value(
        c.post(net(&s, n, &format!("members/{admin_id}/role")))
            .bearer_auth(owner.token.expose())
            .json(&ChangeMemberRoleRequest {
                expected_version: Version::MIN,
                role: MembershipRole::Member,
            }),
    )
    .await;
    // Demotion revokes the admin's unused links; inspect says so, as the
    // list does, instead of pointing at the issuer.
    error(
        inspect(member.token.expose(), &admin_secret)
            .send()
            .await
            .unwrap(),
        StatusCode::BAD_REQUEST,
        "join_link_revoked",
    )
    .await;
    let after_demotion: JoinLinkListResponse = value(
        c.get(net(&s, n, "join-links?limit=10"))
            .bearer_auth(owner.token.expose()),
    )
    .await;
    assert!(
        after_demotion
            .items
            .iter()
            .find(|link| link.id == admin_link.id)
            .unwrap()
            .revoked_at
            .is_some()
    );
    reject(
        c.get(net(&s, n, &format!("join-links?cursor={cursor}")))
            .bearer_auth(admin.token.expose()),
        StatusCode::FORBIDDEN,
        "permission_denied",
    )
    .await;
    c.post(net(&s, n, &format!("join-links/{}/revoke", owner_link.id)))
        .bearer_auth(owner.token.expose())
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    error(
        inspect(member.token.expose(), &owner_secret)
            .send()
            .await
            .unwrap(),
        StatusCode::BAD_REQUEST,
        "join_link_revoked",
    )
    .await;
    let consumed: JoinLinkCreated = value(make(owner.token.expose(), MembershipRole::Member)).await;
    let JoinLinkCreated::Available { secret, .. } = consumed else {
        panic!()
    };
    let redeem = || {
        c.post(format!("{}/api/v1/join-links/redeem", s.url))
            .bearer_auth(member.token.expose())
            .json(&JoinLinkCredentialRequest {
                token: WireSecret::new(secret.expose().into()),
            })
    };
    let joined: JoinLinkRedeemed = value(redeem()).await;
    let retry: JoinLinkRedeemed = value(redeem()).await;
    assert_eq!(joined.membership_id, retry.membership_id);
    let consumer = |links: &JoinLinkListResponse| {
        links
            .items
            .iter()
            .find(|link| link.consumed_at.is_some() && link.issued_by.user_id == owner.user.id)
            .unwrap()
            .consumed_by
            .clone()
            .unwrap()
    };
    let consumed_list: JoinLinkListResponse = value(
        c.get(net(&s, n, "join-links?limit=10"))
            .bearer_auth(owner.token.expose()),
    )
    .await;
    assert_eq!(consumer(&consumed_list).user_id, member.user.id);
    assert_eq!(consumer(&consumed_list).email, "links-member@example.test");
    error(
        inspect(member.token.expose(), &secret)
            .send()
            .await
            .unwrap(),
        StatusCode::BAD_REQUEST,
        "join_link_used",
    )
    .await;
    let revision = source(&c, &s, n, owner.token.expose()).await.revision;
    c.post(net(
        &s,
        n,
        &format!("members/{}/remove", joined.membership_id),
    ))
    .bearer_auth(owner.token.expose())
    .json(&RemoveMemberRequest {
        expected_version: Version::MIN,
        expected_revision: revision,
    })
    .send()
    .await
    .unwrap()
    .error_for_status()
    .unwrap();
    error(
        redeem().send().await.unwrap(),
        StatusCode::BAD_REQUEST,
        "join_link_used",
    )
    .await;
    // The consumer left the network; the link still names them.
    let after_removal: JoinLinkListResponse = value(
        c.get(net(&s, n, "join-links?limit=10"))
            .bearer_auth(owner.token.expose()),
    )
    .await;
    assert_eq!(consumer(&after_removal).email, "links-member@example.test");
    s.close().await;
}

#[tokio::test]
async fn defaults_replay_preserves_other_admin_changes_and_reports_precise_conflicts() {
    let _serial = ACCOUNT_FLOW.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let s = Server::start(&dir.path().canonicalize().unwrap().join("controller"), true).await;
    let c = client();
    let owner = register(&c, &s, "defaults-owner@example.test", "valid password").await;
    let admin = register(&c, &s, "defaults-admin@example.test", "valid password").await;
    let n = create(&c, &s, owner.token.expose(), "partial defaults")
        .await
        .network
        .id;
    let member = join(
        &c,
        &s,
        n,
        owner.token.expose(),
        admin.token.expose(),
        MembershipRole::Admin,
    )
    .await;
    let initial = source(&c, &s, n, owner.token.expose()).await;
    let op: meshway_configuration::ConfigurationOperation =
        serde_json::from_str(r#"{"type":"replaceDefaults","defaults":{"topology":"deny"}}"#)
            .unwrap();
    let mut concurrent =
        meshway_configuration::NetworkConfigurationInput::from(&initial.configuration);
    concurrent.defaults.dialers.vless.as_mut().unwrap().remark = "concurrent".into();
    concurrent
        .defaults
        .dialers
        .vless
        .as_mut()
        .unwrap()
        .websocket = Some(
        serde_json::from_str(r#"{"headers":{"Authorization":"private concurrent secret"}}"#)
            .unwrap(),
    );
    let first: ConfigurationAppliedResponse = json(
        c.post(net(&s, n, "configuration"))
            .bearer_auth(admin.token.expose())
            .json(&ConfigurationRequest::Document {
                expected_revision: initial.revision,
                request_id: request(),
                source_jsonc: WireSecret::new(serde_json::to_string(&concurrent).unwrap()),
            }),
    )
    .await;
    assert_eq!(first.target_count, 0);
    assert!(first.publication_ids.is_empty());
    let rid = request();
    let stale = ConfigurationRequest::Operations {
        expected_revision: initial.revision,
        request_id: rid,
        ops: vec![op.clone()],
    };
    reject(
        c.post(net(&s, n, "configuration"))
            .bearer_auth(owner.token.expose())
            .json(&stale),
        StatusCode::CONFLICT,
        "revision_conflict",
    )
    .await;
    let replay = ConfigurationRequest::Operations {
        expected_revision: first.revision,
        request_id: rid,
        ops: vec![op],
    };
    let applied: ConfigurationAppliedResponse = json(
        c.post(net(&s, n, "configuration"))
            .bearer_auth(owner.token.expose())
            .json(&replay),
    )
    .await;
    let saved = source(&c, &s, n, owner.token.expose()).await;
    assert_eq!(
        saved.configuration.defaults.dialers,
        concurrent.defaults.dialers
    );
    assert_eq!(
        saved.configuration.defaults.access,
        initial.configuration.defaults.access
    );
    assert_eq!(
        saved.configuration.defaults.topology,
        meshway_configuration::TopologyDefault::Deny
    );
    let different = ConfigurationRequest::Operations {
        expected_revision: first.revision,
        request_id: rid,
        ops: vec![],
    };
    reject(
        c.post(net(&s, n, "configuration"))
            .bearer_auth(owner.token.expose())
            .json(&different),
        StatusCode::CONFLICT,
        "request_id_conflict",
    )
    .await;
    reject(
        c.post(net(&s, n, &format!("members/{member}/remove")))
            .bearer_auth(owner.token.expose())
            .json(&RemoveMemberRequest {
                expected_version: Version::MIN,
                expected_revision: initial.revision,
            }),
        StatusCode::CONFLICT,
        "revision_conflict",
    )
    .await;
    reject(
        c.post(net(&s, n, &format!("members/{member}/remove")))
            .bearer_auth(owner.token.expose())
            .json(&RemoveMemberRequest {
                expected_version: Version::new(2).unwrap(),
                expected_revision: applied.revision,
            }),
        StatusCode::CONFLICT,
        "version_conflict",
    )
    .await;
    let clear: meshway_configuration::ConfigurationOperation =
        serde_json::from_str(r#"{"type":"replaceDefaults","defaults":{"dialers":{}}}"#).unwrap();
    c.post(net(&s, n, "configuration"))
        .bearer_auth(owner.token.expose())
        .json(&ConfigurationRequest::Operations {
            expected_revision: applied.revision,
            request_id: request(),
            ops: vec![clear],
        })
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    assert!(
        source(&c, &s, n, owner.token.expose())
            .await
            .configuration
            .defaults
            .dialers
            .vless
            .is_none()
    );
    s.close().await;
}

#[tokio::test]
async fn historical_actor_email_is_a_snapshot_after_departure_and_email_change() {
    let _serial = ACCOUNT_FLOW.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let s = Server::start(&dir.path().canonicalize().unwrap().join("controller"), true).await;
    let c = client();
    let owner = register(&c, &s, "history-owner@example.test", "valid password").await;
    let member = register(&c, &s, "history-old@example.test", "valid password").await;
    let n = create(&c, &s, owner.token.expose(), "history snapshots")
        .await
        .network
        .id;
    let membership = join(
        &c,
        &s,
        n,
        owner.token.expose(),
        member.token.expose(),
        MembershipRole::Member,
    )
    .await;
    let revision = source(&c, &s, n, owner.token.expose()).await.revision;
    c.post(net(&s, n, &format!("members/{membership}/remove")))
        .bearer_auth(owner.token.expose())
        .json(&RemoveMemberRequest {
            expected_version: Version::MIN,
            expected_revision: revision,
        })
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    c.post(format!("{}/api/v1/account/email", s.url))
        .bearer_auth(member.token.expose())
        .json(&ChangeEmailRequest {
            password: WireSecret::new("valid password".into()),
            email: "history-private-new@example.test".into(),
        })
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let text = c
        .get(net(&s, n, "activity"))
        .bearer_auth(owner.token.expose())
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(!text.contains("history-private-new"));
    let history: ActivityListResponse = serde_json::from_str(&text).unwrap();
    assert!(history.items.iter().any(|event| matches!(&event.actor, ActivityActor::User { user_id, email } if *user_id == member.user.id && email.as_deref() == Some("history-old@example.test"))));
    let unknown: ActivityActor = serde_json::from_value(
        serde_json::json!({"type":"user","userId":member.user.id,"email":null}),
    )
    .unwrap();
    assert!(matches!(unknown, ActivityActor::User { email: None, .. }));
    // The account's own history shows the change as "from A to B"; the
    // previous address is only present on the email change itself.
    let own: ActivityListResponse = value(
        c.get(format!("{}/api/v1/account/activity", s.url))
            .bearer_auth(member.token.expose()),
    )
    .await;
    let change = own
        .items
        .iter()
        .find(|event| event.action == ActivityAction::AccountEmail)
        .unwrap();
    assert!(matches!(
        &change.target,
        ActivityTarget::Account { user_id, previous_email }
            if *user_id == member.user.id
                && previous_email.as_deref() == Some("history-old@example.test")
    ));
    assert!(matches!(
        &change.actor,
        ActivityActor::User { email, .. } if email.as_deref() == Some("history-private-new@example.test")
    ));
    assert!(own.items.iter().all(|event| {
        event.action == ActivityAction::AccountEmail
            || !matches!(
                &event.target,
                ActivityTarget::Account {
                    previous_email: Some(_),
                    ..
                }
            )
    }));
    s.close().await;
}
