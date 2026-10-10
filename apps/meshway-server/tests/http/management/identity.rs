use super::*;

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

