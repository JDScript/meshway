use meshway_protocol::NetworkList;

/// Test controllers are fresh; never pick a network by ordering or name.
pub async fn admin_url(base: &str, admin: &str, suffix: &str) -> String {
    assert!(suffix.starts_with('/') && !suffix.starts_with("/api/"));
    let response = reqwest::Client::new()
        .get(format!("{base}/api/v1/networks"))
        .bearer_auth(admin)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let networks: NetworkList = serde_json::from_slice(&response.bytes().await.unwrap()).unwrap();
    assert_eq!(
        networks.networks.len(),
        1,
        "fixture requires a unique initial network"
    );
    format!(
        "{base}/api/v1/networks/{}{suffix}",
        networks.networks[0].network_id
    )
}
