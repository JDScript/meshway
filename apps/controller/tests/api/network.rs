use super::*;
use meshway_protocol::{AgentDnsSnapshot, NetworkSnapshot, dns_digest};

async fn network(f: &Fixture) -> NetworkSnapshot {
    let (status, body) = request(&f.app, "GET", "/api/v1/network", Some(&f.admin), None).await;
    assert_eq!(status, StatusCode::OK);
    serde_json::from_value(body).unwrap()
}

async fn dns(f: &Fixture, credential: &str) -> AgentDnsSnapshot {
    let (status, body) = request(&f.app, "GET", "/api/v1/agent/dns", Some(credential), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let snapshot: AgentDnsSnapshot = serde_json::from_value(body).unwrap();
    snapshot.payload.validate().unwrap();
    assert_eq!(snapshot.digest, dns_digest(&snapshot.payload).unwrap());
    snapshot
}

#[tokio::test]
async fn network_and_dns_roles_global_cas_inventory_and_revocation_are_real() {
    let f = Fixture::new();
    let token = f.token().await.token;
    let one_credential = local::credential().unwrap();
    let one = enroll(&f, &token, &one_credential, "One").await.node;
    let two_credential = local::credential().unwrap();
    let two = enroll(&f, &f.token().await.token, &two_credential, "one")
        .await
        .node;
    let unique_credential = local::credential().unwrap();
    let unique = enroll(&f, &f.token().await.token, &unique_credential, "BÜCHER")
        .await
        .node;
    let initial = network(&f).await;
    assert!(dns(&f, &one_credential).await.payload.records.is_empty());
    let save = json!({"expected_revision":initial.revision,"name":"Meshway", "dns":{"enabled":true,"zone":"MESHWAY.TEST.","ttl_seconds":30}});
    for credential in [
        None,
        Some("invalid"),
        Some(token.as_str()),
        Some(one_credential.as_str()),
    ] {
        assert_eq!(
            request(&f.app, "GET", "/api/v1/network", credential, None)
                .await
                .0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            request(
                &f.app,
                "POST",
                "/api/v1/network",
                credential,
                Some(save.clone())
            )
            .await
            .0,
            StatusCode::UNAUTHORIZED
        );
    }
    for credential in [
        None,
        Some("invalid"),
        Some(token.as_str()),
        Some(f.admin.as_str()),
    ] {
        assert_eq!(
            request(&f.app, "GET", "/api/v1/agent/dns", credential, None)
                .await
                .0,
            StatusCode::UNAUTHORIZED
        );
    }
    let (status, body) = request(
        &f.app,
        "POST",
        "/api/v1/network",
        Some(&f.admin),
        Some(save.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let saved: NetworkSnapshot = serde_json::from_value(body).unwrap();
    assert_eq!(saved.revision, initial.revision + 1);
    assert_eq!(saved.network, initial.network);
    assert_eq!(saved.network_id, initial.network_id);
    assert_eq!(saved.dns.zone, "meshway.test");
    assert_eq!(
        request(
            &f.app,
            "POST",
            "/api/v1/network",
            Some(&f.admin),
            Some(save.clone())
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let mut noop = save.clone();
    noop["expected_revision"] = saved.revision.into();
    let (_, body) = request(
        &f.app,
        "POST",
        "/api/v1/network",
        Some(&f.admin),
        Some(noop),
    )
    .await;
    assert_eq!(body["revision"], saved.revision);
    let bundle = dns(&f, &one_credential).await;
    assert_eq!(bundle.payload.node_id, one.id);
    assert_eq!(bundle.payload.network_id, saved.network_id);
    assert_eq!(bundle.payload.records.len(), 3); // All have no public key; DNS still lists their assigned addresses.
    for id in [&one.id, &two.id] {
        assert!(
            bundle
                .payload
                .records
                .iter()
                .find(|r| &r.node_id == id)
                .unwrap()
                .aliases
                .is_empty()
        );
    }
    assert_eq!(
        bundle
            .payload
            .records
            .iter()
            .find(|r| r.node_id == unique.id)
            .unwrap()
            .aliases,
        ["xn--bcher-kva.meshway.test"]
    );
    let encoded = serde_json::to_string(&bundle).unwrap();
    for secret in [
        &f.admin,
        &token,
        &one_credential,
        &two_credential,
        &unique_credential,
    ] {
        assert!(!encoded.contains(secret));
    }
    request(
        &f.app,
        "POST",
        "/api/v1/agents/heartbeat",
        Some(&one_credential),
        Some(json!({"agent_version":"v2"})),
    )
    .await;
    assert_eq!(dns(&f, &one_credential).await.digest, bundle.digest);
    request(
        &f.app,
        "POST",
        "/api/v1/nodes/revoke",
        Some(&f.admin),
        Some(json!({"node_id":two.id})),
    )
    .await;
    assert_eq!(
        request(
            &f.app,
            "GET",
            "/api/v1/agent/dns",
            Some(&two_credential),
            None
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let after = dns(&f, &one_credential).await;
    assert_eq!(after.payload.records.len(), 2);
    assert_ne!(after.digest, bundle.digest);
    assert_eq!(
        after
            .payload
            .records
            .iter()
            .find(|r| r.node_id == one.id)
            .unwrap()
            .aliases,
        ["one.meshway.test"]
    );
    assert!(!after.payload.records.iter().any(|r| r.node_id == two.id));
}

#[tokio::test]
async fn invalid_network_intent_and_address_pool_edits_never_mutate() {
    let f = Fixture::new();
    let initial = network(&f).await;
    let source = json!({"expected_revision":initial.revision,"name":"Meshway","dns":{"enabled":false,"zone":"meshway.test","ttl_seconds":30}});
    for (field, value) in [
        ("ipv4_prefix", json!("100.97.0.0/16")),
        ("network_id", json!("a".repeat(32))),
        (
            "network",
            json!({"ipv4_prefix":"100.97.0.0/16","ipv6_prefix":"fd00::/64"}),
        ),
    ] {
        let mut invalid = source.clone();
        invalid[field] = value;
        // The shared JSON rejection handler maps closed-schema mismatches to 400.
        assert_eq!(
            request(
                &f.app,
                "POST",
                "/api/v1/network",
                Some(&f.admin),
                Some(invalid)
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    for name in [" ".to_owned(), "x".repeat(129)] {
        let mut invalid = source.clone();
        invalid["name"] = name.into();
        assert_eq!(
            request(
                &f.app,
                "POST",
                "/api/v1/network",
                Some(&f.admin),
                Some(invalid)
            )
            .await
            .0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    for zone in [
        "localhost",
        "127.0.0.1",
        "ip6.arpa",
        "foo..test",
        "*.test",
        "https://example.test",
    ] {
        let mut invalid = source.clone();
        invalid["dns"]["zone"] = zone.into();
        assert_eq!(
            request(
                &f.app,
                "POST",
                "/api/v1/network",
                Some(&f.admin),
                Some(invalid)
            )
            .await
            .0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    for ttl in [0, 301] {
        let mut invalid = source.clone();
        invalid["dns"]["ttl_seconds"] = ttl.into();
        assert_eq!(
            request(
                &f.app,
                "POST",
                "/api/v1/network",
                Some(&f.admin),
                Some(invalid)
            )
            .await
            .0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    assert_eq!(
        serde_json::to_value(network(&f).await).unwrap(),
        serde_json::to_value(initial).unwrap()
    );
}

#[test]
fn openapi_dns_contract_is_closed_bounded_and_role_scoped() {
    let document: Value = serde_json::from_str(&openapi_json().unwrap()).unwrap();
    assert_eq!(
        document["paths"]["/api/v1/agent/dns"]["get"]["operationId"],
        "getAgentDns"
    );
    assert_eq!(
        document["paths"]["/api/v1/agent/dns"]["get"]["security"],
        json!([{"nodeBearer":[]}])
    );
    assert_eq!(
        document["paths"][contract_path("/api/v1/network")]["post"]["security"],
        json!([{"adminBearer":[]}])
    );
    for component in [
        "MeshDnsConfiguration",
        "SaveNetworkRequest",
        "NetworkSnapshot",
        "NodeDnsRecord",
        "NodeDnsConfiguration",
        "AgentDnsSnapshot",
    ] {
        assert_eq!(
            document["components"]["schemas"][component]["additionalProperties"],
            false
        );
    }
    assert_eq!(
        document["components"]["schemas"]["NodeDnsRecord"]["properties"]["aliases"]["maxItems"],
        1
    );
    assert_eq!(
        document["components"]["schemas"]["NodeDnsConfiguration"]["properties"]["records"]["maxItems"],
        1024
    );
    assert_eq!(
        document["components"]["schemas"]["MeshDnsConfiguration"]["properties"]["ttl_seconds"]["maximum"],
        300
    );
}
