mod support;

use meshway_control_client::{
    ClientError, GetNetworkPath, ListNetworkMembersPath, ListNetworkMembersQuery,
    ListNetworksQuery, RenameNetworkPath,
};
use meshway_protocol::{
    CreateNetworkRequest, MembershipRole, Problem, RegisterRequest, RenameNetworkRequest,
    WireSecret,
};

#[tokio::test]
async fn generated_network_operations_preserve_tenant_boundaries_cas_and_pagination() {
    let (app, client, server, _directory) = support::controller(true).await;
    let alice = client
        .register(&RegisterRequest {
            email: "alice@networks.example".into(),
            password: WireSecret::new("valid password alice".into()),
        })
        .await
        .unwrap();
    let bob = client
        .register(&RegisterRequest {
            email: "bob@networks.example".into(),
            password: WireSecret::new("valid password bob".into()),
        })
        .await
        .unwrap();
    let token = alice.token.expose();
    let mut first_id = None;
    for number in 1..=3 {
        let request = CreateNetworkRequest {
            request_id: format!("00000000-0000-4000-8000-{number:012x}")
                .parse()
                .unwrap(),
            name: format!("Network {number}"),
        };
        let created = client.create_network(&request, token).await.unwrap();
        assert_eq!(created.network.role, MembershipRole::Owner);
        assert!(created.network.actions.rename);
        assert_eq!(created.network.current_revision.get(), 1);
        let recovered = client.create_network(&request, token).await.unwrap();
        assert_eq!(created.network, recovered.network);
        if first_id.is_none() {
            first_id = Some(created.network.id);
        }
    }
    let id = first_id.unwrap();
    let detail = client
        .get_network(&GetNetworkPath { network_id: id }, token)
        .await
        .unwrap();
    assert_eq!(detail.network.name, "Network 1");
    assert!(
        matches!(client.get_network(&GetNetworkPath { network_id: id }, bob.token.expose()).await,
        Err(ClientError::Http {status:404,problem:Some(error)}) if matches!(error.problem,Problem::NotFound {..}))
    );
    let renamed = client
        .rename_network(
            &RenameNetworkPath { network_id: id },
            &RenameNetworkRequest {
                request_id: "00000000-0000-4000-8000-000000000010".parse().unwrap(),
                name: "Renamed".into(),
                expected_name: "Network 1".into(),
            },
            token,
        )
        .await
        .unwrap();
    assert_eq!(renamed.network.name, "Renamed");
    assert_eq!(renamed.network.current_revision.get(), 1);
    assert!(
        matches!(client.rename_network(&RenameNetworkPath {network_id:id}, &RenameNetworkRequest {
        request_id: "00000000-0000-4000-8000-000000000011".parse().unwrap(),
        name:"Lost update".into(),expected_name:"Network 1".into(),
    },token).await,Err(ClientError::Http {status:409,problem:Some(error)}) if matches!(error.problem,Problem::NameConflict {..}))
    );
    let members = client
        .list_network_members(
            &ListNetworkMembersPath { network_id: id },
            &ListNetworkMembersQuery::default(),
            token,
        )
        .await
        .unwrap();
    assert_eq!(members.items.len(), 1);
    assert_eq!(members.total.get(), 1);
    assert!(!members.items[0].actions.change_role);
    assert_eq!(
        members.items[0].actions.assignable_roles,
        [MembershipRole::Owner]
    );
    assert!(!members.items[0].actions.remove);
    assert_eq!(members.items[0].user_id, alice.user.id);
    assert_eq!(members.items[0].role, MembershipRole::Owner);
    assert_eq!(members.items[0].membership_id, detail.network.membership_id);

    let page = client
        .list_networks(
            &ListNetworksQuery {
                limit: Some(1),
                cursor: None,
            },
            token,
        )
        .await
        .unwrap();
    let cursor = page.next_cursor.unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.total.get(), 3);
    // New creation is outside the existing cursor's fixed creation boundary.
    client
        .create_network(
            &CreateNetworkRequest {
                request_id: "00000000-0000-4000-8000-000000000004".parse().unwrap(),
                name: "Later".into(),
            },
            token,
        )
        .await
        .unwrap();
    let rest = client
        .list_networks(
            &ListNetworksQuery {
                limit: Some(200),
                cursor: Some(cursor.clone()),
            },
            token,
        )
        .await
        .unwrap();
    assert_eq!(rest.items.len(), 2);
    assert_eq!(rest.total.get(), 3);
    assert!(rest.next_cursor.is_none());
    assert_eq!(page.as_of, rest.as_of);
    assert!(rest.items.iter().all(|item| item.name != "Later"));
    assert!(
        matches!(client.list_networks(&ListNetworksQuery {limit:Some(1),cursor:Some(cursor)}, bob.token.expose()).await,
        Err(ClientError::Http {status:400,problem:Some(error)}) if matches!(error.problem,Problem::InvalidCursor {..}))
    );
    assert!(
        client
            .list_networks(&ListNetworksQuery::default(), bob.token.expose())
            .await
            .unwrap()
            .items
            .is_empty()
    );
    assert!(matches!(
        client
            .list_networks(
                &ListNetworksQuery {
                    limit: Some(0),
                    cursor: None
                },
                token
            )
            .await,
        Err(ClientError::Request)
    ));

    client.logout(token).await.unwrap();
    assert!(matches!(
        client
            .get_network(&GetNetworkPath { network_id: id }, token)
            .await,
        Err(ClientError::Http { status: 401, .. })
    ));
    app.check_storage().await.unwrap();
    server.0.abort();
    app.shutdown().await.unwrap();
}
