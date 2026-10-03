use super::*;
use meshway_protocol::{MeshExecutionResponse, NetworkSnapshot};

const EXECUTION: &str = "/api/v1/agents/mesh-execution";
const ACK: &str = "/api/v1/agents/mesh-execution/ack";

#[tokio::test]
async fn execution_and_receipts_enforce_network_credentials_runtime_and_revocation() {
    let f = Fixture::new();
    // Identical runtime identifiers and numeric revisions must not permit a
    // receipt for one network's execution to be accepted by another network.
    let instance = local::credential().unwrap();
    let runtime =
        json!({"epoch":1,"instance_id":instance,"capabilities":["ipv4_stateful_exact_path_v2"]});
    let mut participants = Vec::new();
    for (index, name) in ["first", "second"].iter().enumerate() {
        let (status, network) = request(
            &f.app,
            "POST",
            "/api/v1/networks",
            Some(&f.admin),
            Some(json!({"name":name})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let network: NetworkSnapshot = serde_json::from_value(network).unwrap();
        let scope = network.network_id;
        let (status, token) = request(
            &f.app,
            "POST",
            &format!("/api/v1/networks/{scope}/enrollment-tokens"),
            Some(&f.admin),
            Some(json!({"expires_in_seconds":600})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let token: CreateEnrollmentTokenResponse = serde_json::from_value(token).unwrap();
        let credential = local::credential().unwrap();
        let node = enroll(&f, &token.token, &credential, name).await.node;
        for (path, body) in [
            (
                "/api/v1/agent/connection-identity",
                json!({"public_key":format!("{:02x}{}",9+index,"00".repeat(31)),"capabilities":[]}),
            ),
            (
                "/api/v1/agent/connections/runtime/begin",
                json!({"expected_epoch":0,"instance_id":instance}),
            ),
        ] {
            assert_eq!(
                request(&f.app, "POST", path, Some(&credential), Some(body))
                    .await
                    .0,
                StatusCode::OK
            );
        }
        let (status, execution) = request(
            &f.app,
            "POST",
            EXECUTION,
            Some(&credential),
            Some(runtime.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let execution: MeshExecutionResponse = serde_json::from_value(execution).unwrap();
        let payload = &execution.snapshot.payload;
        assert_eq!(payload.network_id, scope);
        assert_eq!(payload.node_id, node.id);
        assert_eq!(payload.node.id, node.id);
        assert_eq!(payload.revision, execution.revision);
        assert!(payload.peers.is_empty());
        assert_eq!(
            execution.snapshot.digest,
            meshway_protocol::mesh_execution_digest(payload).unwrap()
        );
        participants.push((scope, node.id, credential, token.token, execution));
    }
    let (scope, node, credential, token, first) = &participants[0];
    let (_, _, other, _, second) = &participants[1];
    assert_eq!(first.revision, second.revision);
    assert_ne!(first.snapshot.digest, second.snapshot.digest);
    let ack = json!({"runtime":runtime,"revision":first.revision,"digest":first.snapshot.digest});
    assert_eq!(
        request(&f.app, "POST", ACK, Some(other), Some(ack.clone()))
            .await
            .0,
        StatusCode::CONFLICT
    );
    let (status, receipt) = request(&f.app, "POST", ACK, Some(credential), Some(ack.clone())).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        receipt,
        json!({"revision":first.revision,"digest":first.snapshot.digest,"saved":true})
    );
    for bearer in [None, Some(f.admin.as_str()), Some(token.as_str())] {
        for (path, body) in [(EXECUTION, &runtime), (ACK, &ack)] {
            assert_eq!(
                request(&f.app, "POST", path, bearer, Some(body.clone()))
                    .await
                    .0,
                StatusCode::UNAUTHORIZED
            );
        }
    }
    let next_instance = local::credential().unwrap();
    assert_eq!(
        request(
            &f.app,
            "POST",
            "/api/v1/agent/connections/runtime/begin",
            Some(credential),
            Some(json!({"expected_epoch":1,"instance_id":next_instance}))
        )
        .await
        .0,
        StatusCode::OK
    );
    for (path, body) in [(EXECUTION, &runtime), (ACK, &ack)] {
        assert_eq!(
            request(&f.app, "POST", path, Some(credential), Some(body.clone()))
                .await
                .0,
            StatusCode::CONFLICT
        );
    }
    let current = json!({"epoch":2,"instance_id":next_instance,"capabilities":["ipv4_stateful_exact_path_v2"]});
    let (status, execution) = request(
        &f.app,
        "POST",
        EXECUTION,
        Some(credential),
        Some(current.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let execution: MeshExecutionResponse = serde_json::from_value(execution).unwrap();
    let current_ack =
        json!({"runtime":current,"revision":execution.revision,"digest":execution.snapshot.digest});
    assert_eq!(
        request(
            &f.app,
            "POST",
            &format!("/api/v1/networks/{scope}/nodes/revoke"),
            Some(&f.admin),
            Some(json!({"node_id":node}))
        )
        .await
        .0,
        StatusCode::OK
    );
    for (path, body) in [(EXECUTION, current), (ACK, current_ack)] {
        assert_eq!(
            request(&f.app, "POST", path, Some(credential), Some(body))
                .await
                .0,
            StatusCode::FORBIDDEN
        );
    }
}
