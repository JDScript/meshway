use super::*;
use meshway_configuration::ConfigurationOperation;
use meshway_model::{MembershipRole, RequestId, Revision};
use meshway_protocol::{
    CreateNetworkRequest, NetworkNameRules, NetworkResponse, configuration::*, history::*,
    management::*,
};

fn request() -> RequestId {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    RequestId::new(uuid::Uuid::from_u128(u128::from(
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
    )))
    .unwrap()
}

async fn source(
    c: &Client,
    s: &Server,
    network: meshway_model::NetworkId,
    token: &str,
) -> ConfigurationSourceResponse {
    c.get(net(s, network, "configuration/source"))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap()
}

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
    let applied: ConfigurationAppliedResponse = c
        .post(net(&s, n, "configuration"))
        .bearer_auth(owner.token.expose())
        .json(&ConfigurationRequest::Document {
            expected_revision: initial.revision,
            request_id: request(),
            source_jsonc: WireSecret::new(serde_json::to_string(&input).unwrap()),
        })
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
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
    let changed: AccountResponse = c
        .post(format!("{}/api/v1/account/email", s.url))
        .bearer_auth(member.token.expose())
        .json(&ChangeEmailRequest {
            password: WireSecret::new("valid password".into()),
            email: "departed-new@example.test".into(),
        })
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(changed.user.id, member.user.id);
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
        let preview: ConfigurationPreviewResponse = c
            .post(net(&s, n, "configuration/validate"))
            .bearer_auth(owner.token.expose())
            .json(&edit)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(preview.change_count, 0);
        assert!(preview.changes.is_empty());
        assert!(!preview.source_changed);
        assert!(!preview.unavailable_dependencies.is_empty());
        let result: ConfigurationAppliedResponse = c
            .post(net(&s, n, "configuration"))
            .bearer_auth(owner.token.expose())
            .json(&edit)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(result.revision, current.revision);
    }
    let restore = c
        .post(net(&s, n, "configuration"))
        .bearer_auth(owner.token.expose())
        .json(&ConfigurationRequest::Restore {
            expected_revision: current.revision,
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
    let activity: ActivityListResponse = c
        .get(net(&s, n, "activity"))
        .bearer_auth(owner.token.expose())
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(activity.items.iter().any(|event| matches!(event.target, ActivityTarget::Membership { membership_id } if membership_id == old_member)));
    s.close().await;
}

#[tokio::test]
async fn nonsecret_defaults_edit_keeps_and_explicitly_clears_websocket_headers() {
    let _serial = ACCOUNT_FLOW.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let s = Server::start(&dir.path().canonicalize().unwrap().join("controller"), true).await;
    let c = client();
    let owner = register(&c, &s, "headers@example.test", "valid password").await;
    let n = create(&c, &s, owner.token.expose(), "headers")
        .await
        .network
        .id;
    let initial = source(&c, &s, n, owner.token.expose()).await;
    let mut input = meshway_configuration::NetworkConfigurationInput::from(&initial.configuration);
    input.defaults.dialers.vless.as_mut().unwrap().websocket =
        Some(serde_json::from_str(r#"{"headers":{"Authorization":"private value"}}"#).unwrap());
    c.post(net(&s, n, "configuration"))
        .bearer_auth(owner.token.expose())
        .json(&ConfigurationRequest::Document {
            expected_revision: initial.revision,
            request_id: request(),
            source_jsonc: WireSecret::new(serde_json::to_string(&input).unwrap()),
        })
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    for (websocket, expected_count) in [(r#"{}"#, 1), (r#"{"headers":{}}"#, 0)] {
        let current = source(&c, &s, n, owner.token.expose()).await;
        let op = serde_json::from_str(&format!(r#"{{"type":"replaceDefaults","defaults":{{"dialers":{{"vless":{{"priority":17,"websocket":{websocket}}}}}}}}}"#)).unwrap();
        let edit = ConfigurationRequest::Operations {
            expected_revision: current.revision,
            request_id: request(),
            ops: vec![op],
        };
        let preview = c
            .post(net(&s, n, "configuration/validate"))
            .bearer_auth(owner.token.expose())
            .json(&edit)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .text()
            .await
            .unwrap();
        assert!(!preview.contains("private value"));
        c.post(net(&s, n, "configuration"))
            .bearer_auth(owner.token.expose())
            .json(&edit)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
        let saved = source(&c, &s, n, owner.token.expose()).await;
        let vless = saved.configuration.defaults.dialers.vless.unwrap();
        assert_eq!(vless.priority, 17);
        assert_eq!(vless.websocket.unwrap().headers.len(), expected_count);
    }
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
    let promoted: MemberMutationResponse = c
        .post(net(&s, n, &format!("members/{member_id}/role")))
        .bearer_auth(owner.token.expose())
        .json(&ChangeMemberRoleRequest {
            expected_version: meshway_model::Version::MIN,
            role: MembershipRole::Admin,
        })
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let link: JoinLinkCreated = c
        .post(net(&s, n, "join-links"))
        .bearer_auth(owner.token.expose())
        .json(&CreateJoinLinkRequest {
            request_id: request(),
            role: MembershipRole::Member,
            expires_in_seconds: 600,
        })
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
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
    let events: ActivityListResponse = c
        .get(net(&s, n, "activity"))
        .bearer_auth(owner.token.expose())
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
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
async fn create(c: &Client, s: &Server, token: &str, name: &str) -> NetworkResponse {
    let response = c
        .post(format!("{}/api/v1/networks", s.url))
        .bearer_auth(token)
        .json(&CreateNetworkRequest {
            request_id: request(),
            name: name.into(),
        })
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    response.json().await.unwrap()
}
fn net(s: &Server, n: meshway_model::NetworkId, path: &str) -> String {
    format!("{}/api/v1/networks/{n}/{path}", s.url)
}
fn query(url: String, params: &[(&str, &str)]) -> reqwest::Url {
    let mut url = reqwest::Url::parse(&url).unwrap();
    url.query_pairs_mut().extend_pairs(params.iter().copied());
    url
}

async fn join(
    c: &Client,
    s: &Server,
    network: meshway_model::NetworkId,
    issuer: &str,
    recipient: &str,
    role: MembershipRole,
) -> meshway_model::MembershipId {
    let created: JoinLinkCreated = c
        .post(net(s, network, "join-links"))
        .bearer_auth(issuer)
        .json(&CreateJoinLinkRequest {
            request_id: request(),
            role,
            expires_in_seconds: 600,
        })
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let JoinLinkCreated::Available { secret, .. } = created else {
        panic!("missing first delivery")
    };
    c.post(format!("{}/api/v1/join-links/redeem", s.url))
        .bearer_auth(recipient)
        .json(&JoinLinkCredentialRequest {
            token: WireSecret::new(secret.expose().into()),
        })
        .send()
        .await
        .unwrap()
        .json::<JoinLinkRedeemed>()
        .await
        .unwrap()
        .membership_id
}

#[tokio::test]
async fn member_actions_and_creation_bounded_totals_follow_current_permissions() {
    let _serial = ACCOUNT_FLOW.lock().await;
    use meshway_protocol::{NetworkListResponse, NetworkMembersResponse};
    let dir = tempfile::tempdir().unwrap();
    let s = Server::start(&dir.path().canonicalize().unwrap().join("controller"), true).await;
    let c = client();
    let owner = register(&c, &s, "owner-count@example.test", "valid password").await;
    let admin = register(&c, &s, "admin-count@example.test", "valid password").await;
    let member = register(&c, &s, "member-count@example.test", "valid password").await;
    let guest = register(&c, &s, "guest-count@example.test", "valid password").await;
    let network = create(&c, &s, owner.token.expose(), "primary")
        .await
        .network;
    create(&c, &s, owner.token.expose(), "second").await;
    create(&c, &s, owner.token.expose(), "third").await;
    create(&c, &s, member.token.expose(), "isolated").await;
    let list: NetworkListResponse = c
        .get(format!("{}/api/v1/networks?limit=1", s.url))
        .bearer_auth(owner.token.expose())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list.total.get(), 3);
    assert_eq!(list.items.len(), 1);
    let network_cursor = list.next_cursor.unwrap();
    create(&c, &s, owner.token.expose(), "after boundary").await;
    let page: NetworkListResponse = c
        .get(query(
            format!("{}/api/v1/networks", s.url),
            &[("limit", "1"), ("cursor", network_cursor.as_str())],
        ))
        .bearer_auth(owner.token.expose())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(page.total.get(), 3);
    let admin_id = join(
        &c,
        &s,
        network.id,
        owner.token.expose(),
        admin.token.expose(),
        MembershipRole::Admin,
    )
    .await;
    let member_id = join(
        &c,
        &s,
        network.id,
        owner.token.expose(),
        member.token.expose(),
        MembershipRole::Member,
    )
    .await;
    let list: NetworkMembersResponse = c
        .get(net(&s, network.id, "members?limit=1"))
        .bearer_auth(owner.token.expose())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list.total.get(), 3);
    assert_eq!(list.items.len(), 1);
    assert!(!list.items[0].actions.change_role);
    assert!(!list.items[0].actions.remove);
    assert_eq!(
        list.items[0].actions.assignable_roles,
        vec![MembershipRole::Owner]
    );
    let member_cursor = list.next_cursor.unwrap();
    let guest_id = join(
        &c,
        &s,
        network.id,
        owner.token.expose(),
        guest.token.expose(),
        MembershipRole::Member,
    )
    .await;
    let page: NetworkMembersResponse = c
        .get(query(
            net(&s, network.id, "members"),
            &[("limit", "1"), ("cursor", member_cursor.as_str())],
        ))
        .bearer_auth(owner.token.expose())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(page.total.get(), 3);
    assert_eq!(page.items[0].membership_id, admin_id);
    assert!(page.items[0].actions.change_role);
    assert!(page.items[0].actions.remove);
    let admins: NetworkMembersResponse = c
        .get(net(&s, network.id, "members?limit=1"))
        .bearer_auth(admin.token.expose())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(admins.total.get(), 4);
    assert!(!admins.items[0].actions.remove);
    assert!(admins.items[0].actions.assignable_roles.is_empty());
    let admin_cursor = admins.next_cursor.unwrap();
    error(
        c.get(net(&s, network.id, "members"))
            .bearer_auth(member.token.expose())
            .send()
            .await
            .unwrap(),
        StatusCode::FORBIDDEN,
        "permission_denied",
    )
    .await;
    error(
        c.post(net(&s, network.id, &format!("members/{member_id}/role")))
            .bearer_auth(admin.token.expose())
            .json(&ChangeMemberRoleRequest {
                expected_version: meshway_model::Version::MIN,
                role: MembershipRole::Admin,
            })
            .send()
            .await
            .unwrap(),
        StatusCode::FORBIDDEN,
        "permission_denied",
    )
    .await;
    let promoted: MemberMutationResponse = c
        .post(net(&s, network.id, &format!("members/{member_id}/role")))
        .bearer_auth(owner.token.expose())
        .json(&ChangeMemberRoleRequest {
            expected_version: meshway_model::Version::MIN,
            role: MembershipRole::Owner,
        })
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(promoted.member.actions.remove);
    let impact: RemovalImpactResponse = c
        .get(net(&s, network.id, "leave-impact"))
        .bearer_auth(owner.token.expose())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        c.post(net(&s, network.id, "leave"))
            .bearer_auth(owner.token.expose())
            .json(&RemoveMemberRequest {
                expected_version: impact.version,
                expected_revision: impact.revision
            })
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let page: NetworkListResponse = c
        .get(query(
            format!("{}/api/v1/networks", s.url),
            &[("cursor", network_cursor.as_str())],
        ))
        .bearer_auth(owner.token.expose())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(page.total.get(), 2);
    error(
        c.get(query(
            net(&s, network.id, "members"),
            &[("cursor", member_cursor.as_str())],
        ))
        .bearer_auth(owner.token.expose())
        .send()
        .await
        .unwrap(),
        StatusCode::NOT_FOUND,
        "not_found",
    )
    .await;
    let demoted: MemberMutationResponse = c
        .post(net(&s, network.id, &format!("members/{admin_id}/role")))
        .bearer_auth(member.token.expose())
        .json(&ChangeMemberRoleRequest {
            expected_version: meshway_model::Version::MIN,
            role: MembershipRole::Member,
        })
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(demoted.member.version.get(), 2);
    error(
        c.get(query(
            net(&s, network.id, "members"),
            &[("cursor", admin_cursor.as_str())],
        ))
        .bearer_auth(admin.token.expose())
        .send()
        .await
        .unwrap(),
        StatusCode::FORBIDDEN,
        "permission_denied",
    )
    .await;
    let list: NetworkMembersResponse = c
        .get(net(&s, network.id, "members?limit=1"))
        .bearer_auth(member.token.expose())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list.total.get(), 3);
    assert_eq!(
        c.post(net(&s, network.id, &format!("members/{guest_id}/remove")))
            .bearer_auth(member.token.expose())
            .json(&RemoveMemberRequest {
                expected_version: meshway_model::Version::MIN,
                expected_revision: Revision::MIN
            })
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let page: NetworkMembersResponse = c
        .get(query(
            net(&s, network.id, "members"),
            &[("cursor", list.next_cursor.as_deref().unwrap())],
        ))
        .bearer_auth(member.token.expose())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(page.total.get(), 2);
    assert!(page.items.iter().all(|m| m.membership_id != guest_id));
    assert!(meshway_model::Count::new(meshway_model::MAX_SAFE_INTEGER + 1).is_err());
    s.close().await;
}

#[tokio::test]
async fn account_password_sessions_cli_recovery_and_private_history() {
    let _serial = ACCOUNT_FLOW.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().canonicalize().unwrap().join("controller");
    let s = Server::start(&path, true).await;
    let c = client();
    let user = register(&c, &s, "manager@example.test", "valid password").await;
    let other = register(&c, &s, "other@example.test", "valid password").await;
    let second: SessionIssuedResponse = c
        .post(s.auth("login"))
        .json(&LoginRequest {
            email: user.user.email.clone(),
            password: WireSecret::new("valid password".into()),
        })
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let list: AccountSessionsResponse = c
        .get(format!("{}/api/v1/account/sessions", s.url))
        .bearer_auth(user.token.expose())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list.items.len(), 2);
    assert_eq!(list.items.iter().filter(|s| s.current).count(), 1);
    let first: AccountSessionsResponse = c
        .get(format!("{}/api/v1/account/sessions?limit=1", s.url))
        .bearer_auth(user.token.expose())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(first.items[0].id, second.session.id);
    let cursor = first.next_cursor.unwrap();
    let next_url = query(
        format!("{}/api/v1/account/sessions", s.url),
        &[("cursor", &cursor)],
    );
    error(
        c.get(next_url.clone())
            .bearer_auth(other.token.expose())
            .send()
            .await
            .unwrap(),
        StatusCode::BAD_REQUEST,
        "invalid_cursor",
    )
    .await;
    let later: SessionIssuedResponse = c
        .post(s.auth("login"))
        .json(&LoginRequest {
            email: user.user.email.clone(),
            password: WireSecret::new("valid password".into()),
        })
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let next: AccountSessionsResponse = c
        .get(next_url)
        .bearer_auth(user.token.expose())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(next.as_of, first.as_of);
    assert_eq!(next.items.len(), 1);
    assert_eq!(next.items[0].id, user.session.id);
    assert_ne!(next.items[0].id, later.session.id);
    assert!(next.next_cursor.is_none());
    error(
        c.post(format!(
            "{}/api/v1/account/sessions/{}/revoke",
            s.url, other.session.id
        ))
        .bearer_auth(user.token.expose())
        .send()
        .await
        .unwrap(),
        StatusCode::NOT_FOUND,
        "not_found",
    )
    .await;
    assert_eq!(
        c.post(format!("{}/api/v1/account/sessions/revoke-others", s.url))
            .bearer_auth(user.token.expose())
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    error(
        c.get(s.auth("session"))
            .bearer_auth(second.token.expose())
            .send()
            .await
            .unwrap(),
        StatusCode::UNAUTHORIZED,
        "invalid_session",
    )
    .await;
    error(
        c.post(format!("{}/api/v1/account/password", s.url))
            .bearer_auth(user.token.expose())
            .json(&ChangePasswordRequest {
                current_password: WireSecret::new("wrong password".into()),
                password: WireSecret::new("replacement password".into()),
            })
            .send()
            .await
            .unwrap(),
        StatusCode::BAD_REQUEST,
        "invalid_current_password",
    )
    .await;
    assert_eq!(
        c.post(format!("{}/api/v1/account/password", s.url))
            .bearer_auth(user.token.expose())
            .json(&ChangePasswordRequest {
                current_password: WireSecret::new("valid password".into()),
                password: WireSecret::new("replacement password".into())
            })
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    error(
        c.get(s.auth("session"))
            .bearer_auth(user.token.expose())
            .send()
            .await
            .unwrap(),
        StatusCode::UNAUTHORIZED,
        "invalid_session",
    )
    .await;
    s.close().await;
    let issue_reset = || {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_reset-password"))
            .args([
                "--data-dir",
                path.to_str().unwrap(),
                "--email",
                "manager@example.test",
                "--console-origin",
                "https://controller.example",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stderr.is_empty());
        let output = String::from_utf8(output.stdout).unwrap();
        output
            .trim()
            .strip_prefix("https://controller.example/reset-password#token=")
            .unwrap()
            .to_owned()
    };
    let replaced_token = issue_reset();
    let token = issue_reset();
    let s = Server::start(&path, false).await;
    for (token, code) in [
        (replaced_token, "password_reset_replaced"),
        ("invalid-reset-token".into(), "invalid_password_reset"),
    ] {
        error(
            c.post(s.auth("password-reset/inspect"))
                .json(&PasswordResetInspectRequest {
                    token: WireSecret::new(token),
                })
                .send()
                .await
                .unwrap(),
            StatusCode::BAD_REQUEST,
            code,
        )
        .await;
    }
    let inspect: PasswordResetInspection = c
        .post(s.auth("password-reset/inspect"))
        .json(&PasswordResetInspectRequest {
            token: WireSecret::new(token.clone()),
        })
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(inspect.expires_at > chrono::Utc::now());
    assert_eq!(
        c.post(s.auth("password-reset/redeem"))
            .json(&PasswordResetRedeemRequest {
                token: WireSecret::new(token.clone()),
                password: WireSecret::new("after reset password".into())
            })
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    error(
        c.post(s.auth("password-reset/inspect"))
            .json(&PasswordResetInspectRequest {
                token: WireSecret::new(token),
            })
            .send()
            .await
            .unwrap(),
        StatusCode::BAD_REQUEST,
        "password_reset_used",
    )
    .await;
    let login: SessionIssuedResponse = c
        .post(s.auth("login"))
        .json(&LoginRequest {
            email: user.user.email,
            password: WireSecret::new("after reset password".into()),
        })
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let response = c
        .get(format!("{}/api/v1/account/activity", s.url))
        .bearer_auth(login.token.expose())
        .send()
        .await
        .unwrap();
    let text = response.text().await.unwrap();
    assert!(!text.contains("mwr_"));
    assert!(!text.contains("passwordHash"));
    let activity: ActivityListResponse = serde_json::from_str(&text).unwrap();
    assert!(
        activity
            .items
            .iter()
            .any(|a| matches!(a.actor, ActivityActor::System {}))
    );
    assert!(
        activity.items.iter().all(
            |a| !matches!(a.target,ActivityTarget::Account{user_id} if user_id==other.user.id)
        )
    );
    s.close().await;
}

#[tokio::test]
async fn join_roles_configuration_cas_comment_preservation_and_revocation() {
    let _serial = ACCOUNT_FLOW.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let s = Server::start(&dir.path().canonicalize().unwrap().join("controller"), true).await;
    let c = client();
    let owner = register(&c, &s, "owner@example.test", "valid password").await;
    let member = register(&c, &s, "member@example.test", "valid password").await;
    let network = create(&c, &s, owner.token.expose(), "network")
        .await
        .network;
    let outsider = create(&c, &s, member.token.expose(), "other").await.network;
    error(
        c.get(net(&s, network.id, "configuration/source"))
            .bearer_auth(member.token.expose())
            .send()
            .await
            .unwrap(),
        StatusCode::NOT_FOUND,
        "not_found",
    )
    .await;
    let creation = CreateJoinLinkRequest {
        request_id: request(),
        role: MembershipRole::Member,
        expires_in_seconds: 600,
    };
    let response = c
        .post(net(&s, network.id, "join-links"))
        .bearer_auth(owner.token.expose())
        .json(&creation)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let created: JoinLinkCreated = response.json().await.unwrap();
    let JoinLinkCreated::Available { link, secret, .. } = created else {
        panic!("first delivery must contain secret")
    };
    let retry: JoinLinkCreated = c
        .post(net(&s, network.id, "join-links"))
        .bearer_auth(owner.token.expose())
        .json(&creation)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(matches!(retry, JoinLinkCreated::Unavailable { .. }));
    let inspection: JoinLinkInspection = c
        .post(format!("{}/api/v1/join-links/inspect", s.url))
        .bearer_auth(member.token.expose())
        .json(&JoinLinkCredentialRequest {
            token: WireSecret::new(secret.expose().into()),
        })
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(inspection.link.id, link.id);
    let joined: JoinLinkRedeemed = c
        .post(format!("{}/api/v1/join-links/redeem", s.url))
        .bearer_auth(member.token.expose())
        .json(&JoinLinkCredentialRequest {
            token: WireSecret::new(secret.expose().into()),
        })
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(joined.network_id, network.id);
    error(
        c.get(net(&s, network.id, "configuration/source"))
            .bearer_auth(member.token.expose())
            .send()
            .await
            .unwrap(),
        StatusCode::FORBIDDEN,
        "permission_denied",
    )
    .await;
    error(
        c.post(net(
            &s,
            outsider.id,
            &format!("members/{}/role", joined.membership_id),
        ))
        .bearer_auth(owner.token.expose())
        .json(&ChangeMemberRoleRequest {
            expected_version: meshway_model::Version::MIN,
            role: MembershipRole::Admin,
        })
        .send()
        .await
        .unwrap(),
        StatusCode::NOT_FOUND,
        "not_found",
    )
    .await;
    error(
        c.post(net(
            &s,
            network.id,
            &format!("members/{}/role", network.membership_id),
        ))
        .bearer_auth(owner.token.expose())
        .json(&ChangeMemberRoleRequest {
            expected_version: meshway_model::Version::MIN,
            role: MembershipRole::Admin,
        })
        .send()
        .await
        .unwrap(),
        StatusCode::CONFLICT,
        "last_owner",
    )
    .await;
    let promoted: MemberMutationResponse = c
        .post(net(
            &s,
            network.id,
            &format!("members/{}/role", joined.membership_id),
        ))
        .bearer_auth(owner.token.expose())
        .json(&ChangeMemberRoleRequest {
            expected_version: meshway_model::Version::MIN,
            role: MembershipRole::Admin,
        })
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(promoted.member.version.get(), 2);
    let source: ConfigurationSourceResponse = c
        .get(net(&s, network.id, "configuration/source"))
        .bearer_auth(owner.token.expose())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let doc = ConfigurationRequest::Document {
        expected_revision: Revision::MIN,
        request_id: request(),
        source_jsonc: WireSecret::new(format!(
            "// retained comment\n// {}\n{}",
            "long configuration comment ".repeat(1000),
            source.source_jsonc.expose()
        )),
    };
    let applied: ConfigurationAppliedResponse = c
        .post(net(&s, network.id, "configuration"))
        .bearer_auth(owner.token.expose())
        .json(&doc)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(applied.revision.get(), 2);
    assert_eq!(applied.target_count, 0);
    let edit = ConfigurationRequest::Operations {
        expected_revision: applied.revision,
        request_id: request(),
        ops: vec![ConfigurationOperation::ReplaceAccess { access: vec![] }],
    };
    let preview: ConfigurationPreviewResponse = c
        .post(net(&s, network.id, "configuration/validate"))
        .bearer_auth(member.token.expose())
        .json(&edit)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(preview.change_count, 1);
    assert!(preview.preview.access.is_empty());
    let applied: ConfigurationAppliedResponse = c
        .post(net(&s, network.id, "configuration"))
        .bearer_auth(member.token.expose())
        .json(&edit)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(applied.revision.get(), 3);
    let retry: ConfigurationAppliedResponse = c
        .post(net(&s, network.id, "configuration"))
        .bearer_auth(member.token.expose())
        .json(&edit)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(retry.revision, applied.revision);
    error(
        c.post(net(&s, network.id, "configuration/validate"))
            .bearer_auth(member.token.expose())
            .json(&edit)
            .send()
            .await
            .unwrap(),
        StatusCode::CONFLICT,
        "revision_conflict",
    )
    .await;
    let source: ConfigurationSourceResponse = c
        .get(net(&s, network.id, "configuration/source"))
        .bearer_auth(owner.token.expose())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        source
            .source_jsonc
            .expose()
            .starts_with("// retained comment")
    );
    let history: ConfigurationRevisionList = c
        .get(net(&s, network.id, "configuration/revisions?limit=1"))
        .bearer_auth(owner.token.expose())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(history.items[0].revision.get(), 3);
    assert!(history.next_cursor.is_some());
    let restored: ConfigurationAppliedResponse = c
        .post(net(&s, network.id, "configuration"))
        .bearer_auth(owner.token.expose())
        .json(&ConfigurationRequest::Restore {
            expected_revision: applied.revision,
            request_id: request(),
            restore_from: Revision::MIN,
        })
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(restored.revision.get(), 4);
    s.close().await;
}

#[tokio::test]
async fn advertised_network_name_rules_match_unicode_http_boundary() {
    let _serial = ACCOUNT_FLOW.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let s = Server::start(&dir.path().canonicalize().unwrap().join("controller"), true).await;
    let c = client();
    let options: AuthOptionsResponse = c
        .get(s.auth("options"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(options.network_name_rules, NetworkNameRules::CURRENT);
    let user = register(&c, &s, "names@example.test", "valid password").await;
    error(
        c.get(format!("{}/api/v1/configuration/schema", s.url))
            .send()
            .await
            .unwrap(),
        StatusCode::UNAUTHORIZED,
        "invalid_session",
    )
    .await;
    let schema: ConfigurationSchemaResponse = c
        .get(format!("{}/api/v1/configuration/schema", s.url))
        .bearer_auth(user.token.expose())
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let schema: serde_json::Value = serde_json::from_str(&schema.schema_json).unwrap();
    fn references(value: &serde_json::Value, root: &serde_json::Value) {
        match value {
            serde_json::Value::Object(object) => {
                if let Some(reference) = object.get("$ref") {
                    let reference = reference.as_str().unwrap();
                    assert!(reference.starts_with("#/$defs/"));
                    assert!(
                        root.pointer(&reference[1..]).is_some(),
                        "missing schema {reference}"
                    );
                }
                for child in object.values() {
                    references(child, root);
                }
            }
            serde_json::Value::Array(array) => {
                for child in array {
                    references(child, root);
                }
            }
            _ => {}
        }
    }
    references(&schema, &schema);
    let catalog: TransportCatalogResponse = c
        .get(format!("{}/api/v1/transport-catalog", s.url))
        .bearer_auth(user.token.expose())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(catalog.items.len(), 4);
    for entry in catalog.items {
        assert!(schema["$defs"].get(entry.listener_schema).is_some());
        assert!(schema["$defs"].get(entry.dialer_schema).is_some());
        assert!(entry.required_capability.starts_with("transport:"));
    }
    for (raw, normalized) in [
        (
            format!("\u{2003}{}\u{3000}", "😀".repeat(128)),
            "😀".repeat(128),
        ),
        ("\u{feff}name\u{feff}".into(), "\u{feff}name\u{feff}".into()),
        (
            format!("{}x{}", " ".repeat(511), " ".repeat(512)),
            "x".into(),
        ),
    ] {
        assert_eq!(
            create(&c, &s, user.token.expose(), &raw).await.network.name,
            normalized
        );
    }
    for raw in [
        "😀".repeat(129),
        "\tname".into(),
        "name\u{0085}".into(),
        "a\0b".into(),
        format!("{}x", " ".repeat(1024)),
        "\u{2003}\u{3000}".into(),
    ] {
        error(
            c.post(format!("{}/api/v1/networks", s.url))
                .bearer_auth(user.token.expose())
                .json(&CreateNetworkRequest {
                    request_id: request(),
                    name: raw,
                })
                .send()
                .await
                .unwrap(),
            StatusCode::BAD_REQUEST,
            "invalid_request",
        )
        .await;
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
    let source: ConfigurationSourceResponse = c
        .get(net(&s, network.id, "configuration/source"))
        .bearer_auth(owner.token.expose())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
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
    let changed: AccountResponse = c
        .post(format!("{}/api/v1/account/email", s.url))
        .bearer_auth(owner.token.expose())
        .json(&ChangeEmailRequest {
            password: WireSecret::new("valid password".into()),
            email: "after@example.test".into(),
        })
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(changed.user.id, owner.user.id);
    let source: ConfigurationSourceResponse = c
        .get(net(&s, network.id, "configuration/source"))
        .bearer_auth(owner.token.expose())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
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
    let restored: ConfigurationAppliedResponse = c
        .post(net(&s, network.id, "configuration"))
        .bearer_auth(owner.token.expose())
        .json(&ConfigurationRequest::Restore {
            expected_revision: source.revision,
            request_id: request(),
            restore_from: applied.revision,
        })
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let source: ConfigurationSourceResponse = c
        .get(net(&s, network.id, "configuration/source"))
        .bearer_auth(owner.token.expose())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(source.revision, restored.revision);
    assert_eq!(
        source.configuration.groups["group:team"],
        vec!["after@example.test"]
    );
    let original: ConfigurationSourceResponse = c
        .get(net(
            &s,
            network.id,
            &format!("configuration/source?revision={}", applied.revision.get()),
        ))
        .bearer_auth(owner.token.expose())
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        original.configuration.groups["group:team"],
        vec!["before@example.test"]
    );
    s.close().await;
}
