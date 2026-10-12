use super::*;

#[tokio::test]
async fn member_inventory_names_the_mesh_dns_name_and_follows_configuration_changes() {
    let _serial = ACCOUNT_FLOW.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let s = Server::start(&dir.path().canonicalize().unwrap().join("dns"), true).await;
    let c = client();
    let owner = register(&c, &s, "dns-owner@example.test", "valid password").await;
    let member = register(&c, &s, "dns-member@example.test", "valid password").await;
    let network = create(&c, &s, owner.token.expose(), "dns").await.network.id;
    join(
        &c,
        &s,
        network,
        owner.token.expose(),
        member.token.expose(),
        MembershipRole::Member,
    )
    .await;
    let (node, _, _) = enroll(&c, &s, network, member.token.expose(), 7).await;
    let inventory = |token: String| {
        let c = c.clone();
        let url = net(&s, network, "nodes");
        async move {
            let inventory: NodeInventoryResponse = json(c.get(url).bearer_auth(token)).await;
            inventory.items
        }
    };
    let current = source(&c, &s, network, owner.token.expose()).await;
    let hostname = current.configuration.nodes[&node.node_id].hostname.clone();
    let domain = current.configuration.dns.domain.clone();
    let items = inventory(member.token.expose().to_owned()).await;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].dns_name, format!("{hostname}.{domain}"));
    assert!(items[0].magic_dns);
    let mut events =
        Events::open(&c, net(&s, network, "events"), member.token.expose(), None).await;
    loop {
        if let StreamEvent::EntitiesSnapshot { entities, .. } = events.user().await.1.0 {
            assert_eq!(entities.nodes[&node.node_id], items[0]);
            break;
        }
    }
    events.ready().await;
    // Renaming the Node, changing the suffix and turning MagicDNS off reach
    // GET and SSE with the same configuration revision.
    let mut input = current.configuration.clone();
    input.nodes.get_mut(&node.node_id).unwrap().hostname = "renamed".into();
    input.dns.domain = "Mesh.Example.".into();
    input.dns.magic_dns = false;
    let _: ConfigurationAppliedResponse = json(
        c.post(net(&s, network, "configuration"))
            .bearer_auth(owner.token.expose())
            .json(&ConfigurationRequest::Document {
                expected_revision: current.revision,
                request_id: request(),
                source_jsonc: WireSecret::new(serde_json::to_string(&input).unwrap()),
            }),
    )
    .await;
    // The member's own Node settings changed, so the stream may resend its
    // complete batch instead of a delta.
    let updated = loop {
        let entities = match events.user().await.1.0 {
            StreamEvent::EntitiesDelta { updated, .. } => updated,
            StreamEvent::EntitiesSnapshot { entities, .. } => entities,
            _ => continue,
        };
        if let Some(node) = entities.nodes.get(&node.node_id) {
            break node.clone();
        }
    };
    assert_eq!(updated.dns_name, "renamed.mesh.example");
    assert!(!updated.magic_dns);
    assert_eq!(inventory(member.token.expose().to_owned()).await, [updated]);
    drop(events);
    tokio::time::timeout(Duration::from_secs(5), s.close())
        .await
        .unwrap();
}
