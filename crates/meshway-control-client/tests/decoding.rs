mod support;

use axum::{Router, body::Body, http::Response, routing::any};
use meshway_control_client::ClientError;
use meshway_protocol::{ErrorResponse, LoginRequest, RegisterRequest, WireSecret};
use serde_json::{Value, json};

// Malformed responses start from real Controller envelopes. Raw JSON mutations
// below are adversarial test data, never an alternate wire DTO or client path.
async fn responder(
    status: u16,
    media: &'static str,
    body: String,
    budget: usize,
) -> (meshway_control_client::Client, support::Server) {
    support::serve(
        Router::new().fallback(any(move || {
            let body = body.clone();
            async move {
                Response::builder()
                    .status(status)
                    .header("content-type", media)
                    .body(Body::from(body))
                    .unwrap()
            }
        })),
        budget,
    )
    .await
}

#[tokio::test]
async fn real_envelopes_are_strictly_decoded_through_generated_operations() {
    let (app, real, server, _directory) = support::controller(true).await;
    let issued = real
        .register(&RegisterRequest {
            email: "decoding@example.com".into(),
            password: WireSecret::new("test password with length".into()),
        })
        .await
        .unwrap();
    let valid = serde_json::to_value(&issued).unwrap();
    let invalid_login = real
        .login(&LoginRequest {
            email: issued.user.email.clone(),
            password: WireSecret::new("wrong".into()),
        })
        .await;
    let error = match invalid_login {
        Err(ClientError::Http {
            problem: Some(error),
            ..
        }) => serde_json::to_value(error).unwrap(),
        _ => panic!("expected controller error envelope"),
    };
    let request = LoginRequest {
        email: issued.user.email.clone(),
        password: WireSecret::new("unlogged secret".into()),
    };
    let mut cases = Vec::new();
    let mut value = valid.clone();
    value["user"] = json!([valid["user"]["id"], valid["user"]["email"]]);
    cases.push(value);
    let mut value = valid.clone();
    value["session"] = json!([
        valid["session"]["id"],
        valid["session"]["createdAt"],
        valid["session"]["expiresAt"]
    ]);
    cases.push(value);
    for id in [
        "00000000-0000-0000-0000-000000000000",
        "AAAAAAAA-AAAA-AAAA-AAAA-AAAAAAAAAAAA",
    ] {
        let mut value = valid.clone();
        value["user"]["id"] = json!(id);
        cases.push(value);
    }
    let mut value = valid.clone();
    value["user"]["unknown"] = json!("private");
    cases.push(value);
    let mut value = valid.clone();
    value["unknown"] = json!("private");
    cases.push(value);
    let mut value = valid.clone();
    value["session"]["createdAt"] = json!("2026-02-30T00:00:00Z");
    cases.push(value);
    let mut value = valid.clone();
    value["token"] = Value::Null;
    cases.push(value);
    for value in cases {
        let (client, _fault) =
            responder(200, "application/json", value.to_string(), 1024 * 1024).await;
        assert!(matches!(
            client.login(&request).await,
            Err(ClientError::Decode { status: 200 })
        ));
    }
    let mut malformed = Vec::new();
    malformed.push(json!([
        error["requestId"],
        error["status"],
        error["problem"]
    ]));
    let mut value = error.clone();
    value["problem"]["code"] = json!("unknown_code");
    malformed.push(value);
    let mut value = error.clone();
    value["unknown"] = json!("private");
    malformed.push(value);
    let mut value = error.clone();
    value["problem"]["unknown"] = json!("private");
    malformed.push(value);
    let mut value = error.clone();
    value["problem"]["params"] = json!([]);
    malformed.push(value);
    let mut value = error.clone();
    value["status"] = json!(65536);
    malformed.push(value);
    let mut value = error.clone();
    value.as_object_mut().unwrap().remove("requestId");
    malformed.push(value);
    for value in malformed {
        let (client, _fault) =
            responder(401, "application/json", value.to_string(), 1024 * 1024).await;
        assert!(matches!(
            client.login(&request).await,
            Err(ClientError::Http {
                status: 401,
                problem: None
            })
        ));
    }
    let mut nullable = error;
    nullable["requestId"] = Value::Null;
    let (client, _fault) =
        responder(401, "application/json", nullable.to_string(), 1024 * 1024).await;
    assert!(matches!(
        client.login(&request).await,
        Err(ClientError::Http {
            status: 401,
            problem: Some(ErrorResponse {
                request_id: None,
                ..
            })
        })
    ));
    for (status, media, body, budget) in [
        (200, "text/html", valid.to_string(), 4096),
        (200, "application/json", valid.to_string(), 4),
        (401, "application/json", "private invalid body".into(), 4096),
    ] {
        let (client, _fault) = responder(status, media, body, budget).await;
        // Use a bodyless generated operation to isolate response budget checks.
        let result = client.get_auth_session("test-token").await;
        match (status, media, budget) {
            (200, "text/html", _) => {
                assert!(matches!(result, Err(ClientError::Decode { status: 200 })))
            }
            (200, _, 4) => assert!(matches!(
                result,
                Err(ClientError::BodyLimit { status: 200 })
            )),
            _ => assert!(matches!(
                result,
                Err(ClientError::Http {
                    status: 401,
                    problem: None
                })
            )),
        }
    }
    server.0.abort();
    app.shutdown().await.unwrap();
}
