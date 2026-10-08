use super::*;

#[tokio::test]
async fn role_change_impact_previews_exactly_the_links_the_change_revokes() {
    let _serial = ACCOUNT_FLOW.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let s = Server::start(&dir.path().canonicalize().unwrap().join("controller"), true).await;
    let c = client();
    let owner = register(&c, &s, "owner-impact@example.test", "valid password").await;
    let second = register(&c, &s, "second-owner-impact@example.test", "valid password").await;
    let admin = register(&c, &s, "admin-impact@example.test", "valid password").await;
    let network = create(&c, &s, owner.token.expose(), "impact").await.network;
    let second_id = join(
        &c,
        &s,
        network.id,
        owner.token.expose(),
        second.token.expose(),
        MembershipRole::Owner,
    )
    .await;
    join(
        &c,
        &s,
        network.id,
        owner.token.expose(),
        admin.token.expose(),
        MembershipRole::Admin,
    )
    .await;
    // The second owner issues one link per grantable role.
    let issue = |role: MembershipRole| {
        let (c, s, token) = (&c, &s, second.token.expose());
        async move {
            let created: JoinLinkCreated = json(
                c.post(net(s, network.id, "join-links"))
                    .bearer_auth(token)
                    .json(&CreateJoinLinkRequest {
                        request_id: request(),
                        role,
                        expires_in_seconds: 600,
                    }),
            )
            .await;
            let JoinLinkCreated::Available { link, .. } = created else {
                panic!("missing first delivery")
            };
            link.id
        }
    };
    let member_link = issue(MembershipRole::Member).await;
    let admin_link = issue(MembershipRole::Admin).await;
    let owner_link = issue(MembershipRole::Owner).await;
    let impact = |token: &str, role: &str| {
        c.get(net(
            &s,
            network.id,
            &format!("members/{second_id}/role-change-impact/{role}"),
        ))
        .bearer_auth(token)
        .send()
    };
    // Staying owner loses nothing.
    let response: RoleChangeImpactResponse = impact(owner.token.expose(), "owner")
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(response.join_link_count, 0);
    assert!(response.join_links.is_empty());
    assert_eq!(response.membership_id, second_id);
    assert_eq!(response.version, meshway_model::Version::MIN);
    // An admin may only issue member links: demotion to admin loses the rest.
    let response: RoleChangeImpactResponse = impact(owner.token.expose(), "admin")
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(response.role, MembershipRole::Admin);
    assert_eq!(response.join_link_count, 2);
    let mut to_admin: Vec<_> = response.join_links.iter().map(|l| l.id).collect();
    to_admin.sort();
    let mut expected = vec![admin_link, owner_link];
    expected.sort();
    assert_eq!(to_admin, expected);
    assert!(response.join_links.iter().all(|l| l.actions.revoke));
    let response: RoleChangeImpactResponse = impact(owner.token.expose(), "member")
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(response.join_link_count, 3);
    assert_eq!(response.join_links.len(), 3);
    assert!(response.join_links.iter().any(|l| l.id == member_link));
    error(
        impact(owner.token.expose(), "guest").await.unwrap(),
        StatusCode::BAD_REQUEST,
        "invalid_request",
    )
    .await;
    // An admin cannot preview a change it could not make.
    error(
        impact(admin.token.expose(), "member").await.unwrap(),
        StatusCode::FORBIDDEN,
        "permission_denied",
    )
    .await;
    // The preview is read-only: every link is still live afterwards.
    let links: JoinLinkListResponse = json(
        c.get(net(&s, network.id, "join-links"))
            .bearer_auth(owner.token.expose()),
    )
    .await;
    assert!(links.items.iter().all(|l| l.revoked_at.is_none()));
    // The change revokes exactly the previewed links.
    let demoted: MemberMutationResponse = json(
        c.post(net(&s, network.id, &format!("members/{second_id}/role")))
            .bearer_auth(owner.token.expose())
            .json(&ChangeMemberRoleRequest {
                expected_version: meshway_model::Version::MIN,
                role: MembershipRole::Admin,
            }),
    )
    .await;
    assert_eq!(demoted.member.role, MembershipRole::Admin);
    let links: JoinLinkListResponse = json(
        c.get(net(&s, network.id, "join-links"))
            .bearer_auth(owner.token.expose()),
    )
    .await;
    assert_eq!(links.items.len(), 5);
    for link in &links.items {
        assert_eq!(
            link.revoked_at.is_some(),
            to_admin.contains(&link.id),
            "{link:?}"
        );
    }
    s.close().await;
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
    let list: NetworkListResponse = json(
        c.get(format!("{}/api/v1/networks?limit=1", s.url))
            .bearer_auth(owner.token.expose()),
    )
    .await;
    assert_eq!(list.total.get(), 3);
    assert_eq!(list.items.len(), 1);
    let network_cursor = list.next_cursor.unwrap();
    create(&c, &s, owner.token.expose(), "after boundary").await;
    let page: NetworkListResponse = json(
        c.get(query(
            format!("{}/api/v1/networks", s.url),
            &[("limit", "1"), ("cursor", network_cursor.as_str())],
        ))
        .bearer_auth(owner.token.expose()),
    )
    .await;
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
    let list: NetworkMembersResponse = json(
        c.get(net(&s, network.id, "members?limit=1"))
            .bearer_auth(owner.token.expose()),
    )
    .await;
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
    let page: NetworkMembersResponse = json(
        c.get(query(
            net(&s, network.id, "members"),
            &[("limit", "1"), ("cursor", member_cursor.as_str())],
        ))
        .bearer_auth(owner.token.expose()),
    )
    .await;
    assert_eq!(page.total.get(), 3);
    assert_eq!(page.items[0].membership_id, admin_id);
    assert!(page.items[0].actions.change_role);
    assert!(page.items[0].actions.remove);
    let admins: NetworkMembersResponse = json(
        c.get(net(&s, network.id, "members?limit=1"))
            .bearer_auth(admin.token.expose()),
    )
    .await;
    assert_eq!(admins.total.get(), 4);
    assert!(!admins.items[0].actions.remove);
    assert!(admins.items[0].actions.assignable_roles.is_empty());
    let admin_cursor = admins.next_cursor.unwrap();
    reject(
        c.get(net(&s, network.id, "members"))
            .bearer_auth(member.token.expose()),
        StatusCode::FORBIDDEN,
        "permission_denied",
    )
    .await;
    reject(
        c.post(net(&s, network.id, &format!("members/{member_id}/role")))
            .bearer_auth(admin.token.expose())
            .json(&ChangeMemberRoleRequest {
                expected_version: meshway_model::Version::MIN,
                role: MembershipRole::Admin,
            }),
        StatusCode::FORBIDDEN,
        "permission_denied",
    )
    .await;
    let promoted: MemberMutationResponse = json(
        c.post(net(&s, network.id, &format!("members/{member_id}/role")))
            .bearer_auth(owner.token.expose())
            .json(&ChangeMemberRoleRequest {
                expected_version: meshway_model::Version::MIN,
                role: MembershipRole::Owner,
            }),
    )
    .await;
    assert!(promoted.member.actions.remove);
    let impact: RemovalImpactResponse = json(
        c.get(net(&s, network.id, "leave-impact"))
            .bearer_auth(owner.token.expose()),
    )
    .await;
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
    let page: NetworkListResponse = json(
        c.get(query(
            format!("{}/api/v1/networks", s.url),
            &[("cursor", network_cursor.as_str())],
        ))
        .bearer_auth(owner.token.expose()),
    )
    .await;
    assert_eq!(page.total.get(), 2);
    reject(
        c.get(query(
            net(&s, network.id, "members"),
            &[("cursor", member_cursor.as_str())],
        ))
        .bearer_auth(owner.token.expose()),
        StatusCode::NOT_FOUND,
        "not_found",
    )
    .await;
    let demoted: MemberMutationResponse = json(
        c.post(net(&s, network.id, &format!("members/{admin_id}/role")))
            .bearer_auth(member.token.expose())
            .json(&ChangeMemberRoleRequest {
                expected_version: meshway_model::Version::MIN,
                role: MembershipRole::Member,
            }),
    )
    .await;
    assert_eq!(demoted.member.version.get(), 2);
    reject(
        c.get(query(
            net(&s, network.id, "members"),
            &[("cursor", admin_cursor.as_str())],
        ))
        .bearer_auth(admin.token.expose()),
        StatusCode::FORBIDDEN,
        "permission_denied",
    )
    .await;
    let list: NetworkMembersResponse = json(
        c.get(net(&s, network.id, "members?limit=1"))
            .bearer_auth(member.token.expose()),
    )
    .await;
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
    let page: NetworkMembersResponse = json(
        c.get(query(
            net(&s, network.id, "members"),
            &[("cursor", list.next_cursor.as_deref().unwrap())],
        ))
        .bearer_auth(member.token.expose()),
    )
    .await;
    assert_eq!(page.total.get(), 2);
    assert!(page.items.iter().all(|m| m.membership_id != guest_id));
    assert!(meshway_model::Count::new(meshway_model::MAX_SAFE_INTEGER + 1).is_err());
    s.close().await;
}
