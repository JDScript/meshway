use super::*;
use meshway_protocol::{management::*, *};

#[tokio::test]
async fn member_source_ownership_role_revocation_and_tag_conversion_use_current_config() {
    let server = Server::new().await;
    let owner = server.user("owner@example.com").await;
    let member = server.user("member@example.com").await;
    let network = server.network(&owner).await;
    let link: JoinLinkCreated = json(
        server.post(
            &format!("/api/v1/networks/{}/join-links", network.id),
            owner.token.expose(),
            &CreateJoinLinkRequest {
                request_id: id(),
                role: MembershipRole::Member,
                expires_in_seconds: 60,
            },
        ),
        201,
    )
    .await;
    let JoinLinkCreated::Available { secret, .. } = link else {
        panic!()
    };
    let joined: JoinLinkRedeemed = json(
        server.post(
            "/api/v1/join-links/redeem",
            member.token.expose(),
            &JoinLinkCredentialRequest { token: secret },
        ),
        200,
    )
    .await;
    let a = server.enroll(&member, network.id, "member-device").await;
    let b = server.enroll(&owner, network.id, "owner-device").await;
    let other = server.network(&owner).await;
    server
        .configure(&owner, network.id, |cfg| {
            for node in cfg.nodes.values_mut() {
                node.listeners = serde_json::from_value(
                    serde_json::json!({"udp":{"type":"native_udp","listenPort":23456}}),
                )
                .unwrap();
            }
            cfg.tags_owners
                .insert("tag:diag".into(), vec![member.user.email.clone()]);
        })
        .await;
    let endpoint = format!("/api/v1/networks/{}/observations", network.id);
    let own: ObservationTask = json(
        server.post(&endpoint, member.token.expose(), &request(&a, &b)),
        201,
    )
    .await;
    status(
        server.post(&endpoint, member.token.expose(), &request(&b, &a)),
        403,
    )
    .await;
    status(
        server.post(
            &format!("/api/v1/networks/{}/observations", other.id),
            member.token.expose(),
            &request(&a, &b),
        ),
        404,
    )
    .await;
    status(
        server.post(
            &format!("/api/v1/networks/{}/observations", other.id),
            owner.token.expose(),
            &request(&a, &b),
        ),
        403,
    )
    .await;
    let list: ObservationList = json(server.get(&endpoint, owner.token.expose()), 200).await;
    assert!(list.items.is_empty());
    let owner_task: ObservationTask = json(
        server.post(&endpoint, owner.token.expose(), &request(&a, &b)),
        201,
    )
    .await;
    assert_eq!(owner_task.id, own.id);
    status(
        server.post(
            &format!("{endpoint}/{}/cancel", owner_task.subscription_id.unwrap()),
            member.token.expose(),
            &(),
        ),
        204,
    )
    .await;
    let list: ObservationList = json(server.get(&endpoint, owner.token.expose()), 200).await;
    assert_eq!(list.items.len(), 1);
    let promoted: MemberMutationResponse = json(
        server.post(
            &format!(
                "/api/v1/networks/{}/members/{}/role",
                network.id, joined.membership_id
            ),
            owner.token.expose(),
            &ChangeMemberRoleRequest {
                expected_version: Version::MIN,
                role: MembershipRole::Admin,
            },
        ),
        200,
    )
    .await;
    let _: ObservationTask = json(
        server.post(&endpoint, member.token.expose(), &request(&b, &a)),
        201,
    )
    .await;
    let _: MemberMutationResponse = json(
        server.post(
            &format!(
                "/api/v1/networks/{}/members/{}/role",
                network.id, joined.membership_id
            ),
            owner.token.expose(),
            &ChangeMemberRoleRequest {
                expected_version: promoted.member.version,
                role: MembershipRole::Member,
            },
        ),
        200,
    )
    .await;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    server.target(&b).await;
    let tasks: ObservationList =
        json(server.get("/agent-api/v1/tasks", b.token.expose()), 200).await;
    assert!(tasks.items.is_empty());
    server
        .configure(&owner, network.id, |cfg| {
            cfg.nodes.get_mut(&a.id).unwrap().identity = meshway_configuration::NodeIdentity::Tag {
                tags: vec!["tag:diag".into()],
            };
        })
        .await;
    status(
        server.post(&endpoint, member.token.expose(), &request(&a, &b)),
        403,
    )
    .await;
    status(server.get("/agent-api/v1/tasks", a.token.expose()), 401).await;
    server.stop().await;
}
