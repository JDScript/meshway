use super::*;

// Only a shape-valid request is needed to test scope rejection. Domain validity
// and successful mutations remain covered by their owning endpoint suites.
fn request_shape(api: &Value, schema: &Value) -> Value {
    if let Some(reference) = schema["$ref"].as_str() {
        return request_shape(
            api,
            api.pointer(reference.strip_prefix('#').unwrap()).unwrap(),
        );
    }
    if let Some(values) = schema["enum"].as_array() {
        return values[0].clone();
    }
    for union in ["oneOf", "anyOf"] {
        if let Some(variants) = schema[union].as_array() {
            return request_shape(api, &variants[0]);
        }
    }
    let types = &schema["type"];
    if types
        .as_array()
        .is_some_and(|v| v.iter().any(|t| t == "null"))
    {
        return Value::Null;
    }
    let kind = types
        .as_str()
        .or_else(|| types[0].as_str())
        .unwrap_or("object");
    match kind {
        "object" => {
            let mut object = serde_json::Map::new();
            for field in schema["required"].as_array().into_iter().flatten() {
                let field = field.as_str().unwrap();
                let value = match field {
                    "redirect_uri" => json!(meshway_protocol::DEVICE_CALLBACK_URI),
                    "code_challenge" => json!(meshway_protocol::s256(
                        "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"
                    )),
                    "email" => json!("fixture@example.test"),
                    "password" => json!("fixture password secret"),
                    "node_id" | "source_node_id" | "peer_node_id" => json!("a".repeat(64)),
                    name if name.ends_with("_id") => json!("a".repeat(64)),
                    _ => request_shape(api, &schema["properties"][field]),
                };
                object.insert(field.into(), value);
            }
            Value::Object(object)
        }
        "array" => Value::Array(
            (0..schema["minItems"].as_u64().unwrap_or(0))
                .map(|_| request_shape(api, &schema["items"]))
                .collect(),
        ),
        "integer" | "number" => json!(schema["minimum"].as_u64().unwrap_or(1)),
        "boolean" => json!(false),
        "string" => json!(if schema["format"] == "date-time" {
            "2026-10-02T00:00:00Z".into()
        } else {
            "a".repeat(schema["minLength"].as_u64().unwrap_or(1) as usize)
        }),
        "null" => Value::Null,
        other => panic!("unsupported fixture schema {other}"),
    }
}

#[tokio::test]
async fn every_network_route_rechecks_scope_and_membership_under_interleaved_requests() {
    let f = Fixture::new();
    let (_, list) = request(&f.app, "GET", "/api/v1/networks", Some(&f.admin), None).await;
    let first = list["networks"][0]["network_id"].as_str().unwrap();
    let session = f.user_for(first).await;
    let (_, second) = request(
        &f.app,
        "POST",
        "/api/v1/networks",
        Some(&f.admin),
        Some(json!({"name":"isolated"})),
    )
    .await;
    let second = second["network_id"].as_str().unwrap();
    let api: Value = serde_json::from_str(&openapi_json().unwrap()).unwrap();
    let mut checked = 0;
    for (template, item) in api["paths"].as_object().unwrap() {
        if !template.starts_with("/api/v1/networks/{network_id}/") {
            continue;
        }
        for method in ["get", "post"] {
            if item.get(method).is_none() {
                continue;
            }
            let body = (method == "post").then(|| {
                request_shape(
                    &api,
                    &item[method]["requestBody"]["content"]["application/json"]["schema"],
                )
            });
            let path = template
                .replace("{network_id}", second)
                .replace("{node_id}", &"0".repeat(64))
                .replace("{invitation_id}", &"0".repeat(64));
            let denied = request(
                &f.app,
                &method.to_uppercase(),
                &path,
                Some(&session),
                body.clone(),
            )
            .await;
            if template.ends_with("/owner-claim") {
                assert_eq!(denied.0, StatusCode::UNAUTHORIZED);
                checked += 1;
                continue;
            }
            assert_eq!(
                denied.0,
                StatusCode::FORBIDDEN,
                "{method} {template}: {}",
                denied.1
            );
            let missing = template
                .replace("{network_id}", &"f".repeat(32))
                .replace("{node_id}", &"0".repeat(64))
                .replace("{invitation_id}", &"0".repeat(64));
            let response = request(
                &f.app,
                &method.to_uppercase(),
                &missing,
                Some(&session),
                body,
            )
            .await;
            assert_eq!(
                response.0,
                StatusCode::NOT_FOUND,
                "{method} {template}: {}",
                response.1
            );
            let (_, current) = request(
                &f.app,
                "GET",
                &format!("/api/v1/networks/{first}/policy"),
                Some(&session),
                None,
            )
            .await;
            assert_eq!(current["network_id"], first);
            assert_eq!(current["revision"], 1);
            checked += 1;
        }
    }
    assert!(checked > 25, "route coverage unexpectedly small: {checked}");
    let (_, other) = request(
        &f.app,
        "GET",
        &format!("/api/v1/networks/{second}/policy"),
        Some(&f.admin),
        None,
    )
    .await;
    assert_eq!(other["network_id"], second);
    assert_eq!(
        other["policy"],
        serde_json::to_value(meshway_policy::NetworkPolicy::default_template()).unwrap()
    );
}
