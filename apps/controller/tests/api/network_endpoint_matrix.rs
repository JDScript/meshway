use super::*;
use std::collections::BTreeSet;

fn operation_shape(path: &str) -> String {
    let path = path.split('?').next().unwrap();
    let mut parts: Vec<_> = path.split('/').collect();
    if parts.len() > 4 && parts[3] == "networks" {
        parts[4] = "{network_id}";
    }
    parts.join("/")
}

#[tokio::test]
async fn every_declared_endpoint_is_scoped_under_global_admin_mutations_node_requests_and_failures()
{
    let f = Fixture::new();
    let mut covered = BTreeSet::new();
    macro_rules! call {
        ($method:expr,$path:expr,$token:expr,$body:expr) => {{
            let path = $path.to_string();
            covered.insert(($method.to_ascii_lowercase(), operation_shape(&path)));
            request(&f.app, $method, &path, $token, $body).await
        }};
    }
    let (status, initial) = call!("GET", "/api/v1/networks", Some(&f.admin), None);
    assert_eq!(status, StatusCode::OK);
    let mut networks = vec![
        initial["networks"][0]["network_id"]
            .as_str()
            .unwrap()
            .to_owned(),
    ];
    for name in ["B", "C"] {
        let (status, body) = call!(
            "POST",
            "/api/v1/networks",
            Some(&f.admin),
            Some(json!({"name":name}))
        );
        assert_eq!(status, StatusCode::OK);
        networks.push(body["network_id"].as_str().unwrap().to_owned());
    }
    let bases: Vec<_> = networks
        .iter()
        .map(|id| format!("/api/v1/networks/{id}"))
        .collect();
    let capability = json!([{"transport":"native_udp","semantics":"datagram","can_dial":true,"can_accept":true}]);
    let mut nodes = Vec::new();
    let mut credentials = Vec::new();
    let mut public_keys = Vec::new();
    for (network, base) in bases.iter().enumerate() {
        let mut members = Vec::new();
        let mut secrets = Vec::new();
        let mut keys = Vec::new();
        for slot in 0..if network == 0 { 1 } else { 2 } {
            let (status, token) = call!(
                "POST",
                format!("{base}/enrollment-tokens"),
                Some(&f.admin),
                Some(json!({"expires_in_seconds":600}))
            );
            assert_eq!(status, StatusCode::OK);
            let credential = local::credential().unwrap();
            let (status, node) = call!(
                "POST",
                "/api/v1/agents/enroll",
                token["token"].as_str(),
                Some(enrollment(&credential, &format!("member-{network}-{slot}")))
            );
            assert_eq!(status, StatusCode::OK);
            let id = node["node"]["id"].as_str().unwrap().to_owned();
            let key = format!("{:02x}{}", 9 + network * 2 + slot, "00".repeat(31));
            let (status, _) = call!(
                "POST",
                "/api/v1/agent/connection-identity",
                Some(&credential),
                Some(json!({"public_key":key,"capabilities":capability}))
            );
            assert_eq!(status, StatusCode::OK);
            members.push(id);
            secrets.push(credential);
            keys.push(key);
        }
        nodes.push(members);
        credentials.push(secrets);
        public_keys.push(keys);
        if network == 0 {
            continue;
        }
        for suffix in [
            "overview",
            "nodes",
            "network",
            "tags",
            "access",
            "policy",
            "connections",
            "connections/runtime",
            "connections/discovery",
        ] {
            assert_eq!(
                call!("GET", format!("{base}/{suffix}"), Some(&f.admin), None).0,
                StatusCode::OK
            );
        }
        let (_, policy) = call!("GET", format!("{base}/policy"), Some(&f.admin), None);
        assert_eq!(call!("POST",format!("{base}/policy/save"),Some(&f.admin),Some(json!({"expected_revision":policy["revision"],"policy":{"default_action":"direct","rules":[]}}))).0,StatusCode::OK);
        let (_, plan) = call!("GET", format!("{base}/connections"), Some(&f.admin), None);
        let source = json!({"listeners":[{"id":"listen","node_id":nodes[network][1],"transport":"native_udp","bind":"0.0.0.0:42000","advertised_endpoints":[{"host":"192.0.2.20","port":42000}],"credential_ref":null}],"candidates":[{"id":"dial","dialer_node_id":nodes[network][0],"listener_id":"listen","endpoint":{"host":"192.0.2.20","port":42000},"priority":0,"credential_ref":null}]});
        assert_eq!(
            call!(
                "POST",
                format!("{base}/connections"),
                Some(&f.admin),
                Some(json!({"expected_revision":plan["revision"],"configuration":source}))
            )
            .0,
            StatusCode::OK
        );
        let (_, access) = call!("GET", format!("{base}/access"), Some(&f.admin), None);
        assert_eq!(call!("POST",format!("{base}/access"),Some(&f.admin),Some(json!({"expected_revision":access["revision"],"policy":{"default_action":"reject","rules":[{"id":"members","source":["*"],"destination":["*"],"action":"allow","bidirectional":true}]}}))).0,StatusCode::OK);
        let (_, tags) = call!("GET", format!("{base}/tags"), Some(&f.admin), None);
        assert_eq!(call!("POST",format!("{base}/tags"),Some(&f.admin),Some(json!({"expected_revision":tags["revision"],"tags":[{"tag":"tag:member","description":format!("only {network}")}]}))).0,StatusCode::OK);
        let (_, tags) = call!("GET", format!("{base}/tags"), Some(&f.admin), None);
        assert_eq!(call!("POST",format!("{base}/nodes/tags"),Some(&f.admin),Some(json!({"expected_revision":tags["revision"],"node_id":nodes[network][1],"tags":["tag:member"]}))).0,StatusCode::OK);
        let (_, saved) = call!("GET", format!("{base}/network"), Some(&f.admin), None);
        let mut dns = saved["dns"].clone();
        dns["enabled"] = true.into();
        assert_eq!(call!("POST",format!("{base}/network"),Some(&f.admin),Some(json!({"expected_revision":saved["revision"],"name":if network==1 {"B"} else {"C"},"dns":dns}))).0,StatusCode::OK);
    }
    let (_, a_before) = call!("GET", format!("{}/nodes", bases[0]), Some(&f.admin), None);
    let (_, b_policy) = call!("GET", format!("{}/policy", bases[1]), Some(&f.admin), None);
    let (_, c_policy) = call!("GET", format!("{}/policy", bases[2]), Some(&f.admin), None);
    assert_eq!(b_policy["revision"], c_policy["revision"]);
    let c = &credentials[2][0];
    let (_, c_config) = call!("GET", "/api/v1/agents/configuration", Some(c), None);
    let (_, c_connections) = call!("GET", "/api/v1/agent/connections", Some(c), None);
    let (_, c_dns) = call!("GET", "/api/v1/agent/dns", Some(c), None);
    let (_, c_dataplane) = call!("GET", "/api/v1/agent/dataplane", Some(c), None);
    assert_eq!(c_config["payload"]["peers"][0]["id"], nodes[2][1]);
    assert_eq!(c_connections["payload"]["peers"][0]["node_id"], nodes[2][1]);
    assert_eq!(
        c_connections["payload"]["peers"][0]["public_key"],
        public_keys[2][1]
    );
    assert_eq!(
        c_dataplane["payload"]["identities"][0]["node_id"],
        nodes[2][1]
    );
    assert_eq!(c_dns["payload"]["network_id"], networks[2]);
    assert_eq!(c_dns["payload"]["records"].as_array().unwrap().len(), 2);
    let (_, epoch) = call!("GET", "/api/v1/agent/connections/runtime", Some(c), None);
    assert_eq!(epoch["epoch"], 0);
    assert_eq!(
        call!(
            "POST",
            "/api/v1/agent/connections/runtime/begin",
            Some(c),
            Some(json!({"instance_id":local::credential().unwrap(),"expected_epoch":0}))
        )
        .0,
        StatusCode::OK
    );
    let c_runtime = json!({"epoch":1,"sequence":1,"revision":c_connections["payload"]["revision"],"digest":c_connections["digest"],"state":"applied","reason":"none","listeners":[],"peers":[]});
    assert_eq!(
        call!(
            "POST",
            "/api/v1/agent/connections/runtime",
            Some(c),
            Some(c_runtime.clone())
        )
        .0,
        StatusCode::OK
    );
    let c_discovery = json!({"epoch":1,"sequence":1,"revision":c_connections["payload"]["revision"],"digest":c_connections["digest"],"state":"disabled","sockets":[]});
    assert_eq!(
        call!(
            "POST",
            "/api/v1/agent/connections/discovery",
            Some(c),
            Some(c_discovery.clone())
        )
        .0,
        StatusCode::OK
    );

    // Independent history events are credential-derived and cannot leak through
    // a scoped query, even when another network has the same numeric revision.
    let measured = chrono::Utc::now();
    let event = json!({
        "measurement_id": local::credential().unwrap(),
        "peer_node_id": nodes[2][1], "direction": "source_to_peer",
        "runtime_epoch": 1, "revision": c_connections["payload"]["revision"],
        "digest": c_connections["digest"], "sampler_generation": local::credential().unwrap(),
        "measured_at": measured, "completed_at": measured + chrono::Duration::milliseconds(2),
        "outcome": "success", "failure_reasons": [], "rtt_us": 1000,
        "channel_associations": [{"transport": "native_udp", "initiator_node_id": nodes[2][0], "listener_id": "listen", "candidate_id": "dial"}]
    });
    let latency = json!({"samples": [event.clone()]});
    assert_eq!(
        call!(
            "POST",
            "/api/v1/agent/latency/samples",
            Some(c),
            Some(latency.clone())
        )
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call!(
            "POST",
            "/api/v1/agent/latency/samples",
            Some(&f.admin),
            Some(latency.clone())
        )
        .0,
        StatusCode::UNAUTHORIZED
    );
    let mut forged = latency.clone();
    forged["source_node_id"] = json!(nodes[1][0]);
    assert_eq!(
        call!(
            "POST",
            "/api/v1/agent/latency/samples",
            Some(c),
            Some(forged)
        )
        .0,
        StatusCode::BAD_REQUEST
    );
    let mut foreign_peer = event;
    foreign_peer["measurement_id"] = json!(local::credential().unwrap());
    foreign_peer["peer_node_id"] = json!(nodes[1][1]);
    assert_eq!(
        call!(
            "POST",
            "/api/v1/agent/latency/samples",
            Some(c),
            Some(json!({"samples": [foreign_peer]}))
        )
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let query = json!({"source_node_id": nodes[2][0], "peer_node_id": nodes[2][1], "from": measured - chrono::Duration::seconds(30), "until": measured + chrono::Duration::seconds(4), "limit": 10, "after": null});
    let (status, history) = call!(
        "POST",
        format!("{}/latency/history/query", bases[2]),
        Some(&f.admin),
        Some(query.clone())
    );
    assert_eq!(status, StatusCode::OK);
    assert_eq!(history["records"].as_array().unwrap().len(), 1);
    assert_eq!(history["records"][0]["network_id"], networks[2]);
    assert_eq!(history["records"][0]["source_node_id"], nodes[2][0]);
    assert_eq!(history["records"][0]["historical"], true);
    assert_eq!(
        call!(
            "POST",
            format!("{}/latency/history/query", bases[2]),
            Some(c),
            Some(query.clone())
        )
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call!(
            "POST",
            format!("{}/latency/history/query", bases[1]),
            Some(&f.admin),
            Some(query.clone())
        )
        .0,
        StatusCode::NOT_FOUND
    );
    let mut b_query = query;
    b_query["source_node_id"] = json!(nodes[1][0]);
    b_query["peer_node_id"] = json!(nodes[1][1]);
    let (status, empty) = call!(
        "POST",
        format!("{}/latency/history/query", bases[1]),
        Some(&f.admin),
        Some(b_query)
    );
    assert_eq!(status, StatusCode::OK);
    assert!(empty["records"].as_array().unwrap().is_empty());

    let (_, b_network) = call!("GET", format!("{}/network", bases[1]), Some(&f.admin), None);
    let (_, b_tags) = call!("GET", format!("{}/tags", bases[1]), Some(&f.admin), None);
    let (_, b_access) = call!("GET", format!("{}/access", bases[1]), Some(&f.admin), None);
    let (_, b_plan) = call!(
        "GET",
        format!("{}/connections", bases[1]),
        Some(&f.admin),
        None
    );
    let b_flow = json!({"source_node_id":nodes[1][0],"destination_node_id":nodes[1][1],"destination_ip":null,"domain":null,"destination_country":null});
    // Exercise each B management write/preview between unrelated global work,
    // A reads and C credential operations. No-op saves intentionally keep the
    // same numeric CAS as C, so a missing scope cannot hide behind a conflict.
    let b_user = f.user_for(bases[1].rsplit('/').next().unwrap()).await;
    for (suffix, body) in [
        (
            "network",
            json!({"expected_revision":b_policy["revision"],"name":b_network["name"],"dns":b_network["dns"]}),
        ),
        (
            "tags",
            json!({"expected_revision":b_policy["revision"],"tags":b_tags["tags"]}),
        ),
        (
            "nodes/tags",
            json!({"expected_revision":b_policy["revision"],"node_id":nodes[1][1],"tags":["tag:member"]}),
        ),
        (
            "nodes/country",
            json!({"expected_revision":b_policy["revision"],"node_id":nodes[1][0],"country":null}),
        ),
        (
            "access",
            json!({"expected_revision":b_policy["revision"],"policy":b_access["policy"]}),
        ),
        (
            "policy/save",
            json!({"expected_revision":b_policy["revision"],"policy":b_policy["policy"]}),
        ),
        (
            "connections",
            json!({"expected_revision":b_policy["revision"],"configuration":b_plan["configuration"]}),
        ),
        (
            "policy/preview",
            json!({"policy":b_policy["policy"],"flow":b_flow}),
        ),
        ("flows/preview", json!({"flow":b_flow})),
        ("enrollment-tokens", json!({"expires_in_seconds":600})),
        (
            "device-authorizations",
            json!({"redirect_uri":meshway_protocol::DEVICE_CALLBACK_URI,"code_challenge":meshway_protocol::s256(&"x".repeat(43)),"name":"pending-B","agent_version":"scope"}),
        ),
    ] {
        assert_eq!(
            call!("GET", format!("{}/nodes", bases[0]), Some(&f.admin), None).1,
            a_before
        );
        assert_eq!(
            call!(
                "POST",
                "/api/v1/networks",
                Some(&f.admin),
                Some(json!({"name":format!("around-{suffix}")}))
            )
            .0,
            StatusCode::OK
        );
        assert_eq!(
            call!(
                "POST",
                format!("{}/{suffix}", bases[1]),
                Some(if suffix == "device-authorizations" {
                    &b_user
                } else {
                    &f.admin
                }),
                Some(body)
            )
            .0,
            StatusCode::OK
        );
        assert_eq!(
            call!("GET", "/api/v1/agents/configuration", Some(c), None).1,
            c_config
        );
        assert_eq!(
            call!("GET", "/api/v1/agent/dataplane", Some(c), None).1,
            c_dataplane
        );
        assert_eq!(
            call!(
                "POST",
                "/api/v1/agent/connections/runtime",
                Some(c),
                Some(c_runtime.clone())
            )
            .0,
            StatusCode::OK
        );
        assert_eq!(
            call!(
                "POST",
                "/api/v1/agent/connections/discovery",
                Some(c),
                Some(c_discovery.clone())
            )
            .0,
            StatusCode::OK
        );
        assert_eq!(
            call!(
                "POST",
                "/api/v1/agents/configuration/ack",
                Some(c),
                Some(
                    json!({"revision":c_config["payload"]["revision"],"digest":c_config["digest"]})
                )
            )
            .0,
            StatusCode::OK
        );
        assert_eq!(call!("POST","/api/v1/agent/connections/ack",Some(c),Some(json!({"revision":c_connections["payload"]["revision"],"digest":c_connections["digest"]}))).0,StatusCode::OK);
        assert_eq!(
            call!("GET", "/api/v1/networks", Some(c), None).0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            call!("GET", format!("{}/policy", bases[2]), Some(&f.admin), None).1,
            c_policy
        );
    }

    // First authenticate, then resolve a real network. An existing NetworkId
    // does not let node credentials act as administrators, even at equal CAS.
    for (method, suffix, body) in [
        ("GET", "nodes", None),
        (
            "POST",
            "tags",
            Some(json!({"expected_revision":b_policy["revision"],"tags":[]})),
        ),
    ] {
        assert_eq!(
            call!(
                method,
                format!("{}/{suffix}", bases[1]),
                Some(c),
                body.clone()
            )
            .0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            call!(
                method,
                format!("/api/v1/networks/{}/{suffix}", "0".repeat(32)),
                Some(c),
                body
            )
            .0,
            StatusCode::UNAUTHORIZED
        );
    }
    // Equal numeric CAS does not make another node/network's digest usable.
    let (_, b_config) = call!(
        "GET",
        "/api/v1/agents/configuration",
        Some(&credentials[1][0]),
        None
    );
    let (_, b_connections) = call!(
        "GET",
        "/api/v1/agent/connections",
        Some(&credentials[1][0]),
        None
    );
    assert_eq!(
        call!(
            "POST",
            "/api/v1/agents/configuration/ack",
            Some(c),
            Some(json!({"revision":b_config["payload"]["revision"],"digest":b_config["digest"]}))
        )
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(call!("POST","/api/v1/agent/connections/ack",Some(c),Some(json!({"revision":b_connections["payload"]["revision"],"digest":b_connections["digest"]}))).0,StatusCode::CONFLICT);
    let mut foreign_report = c_runtime.clone();
    foreign_report["digest"] = b_connections["digest"].clone();
    foreign_report["sequence"] = 2.into();
    assert_eq!(
        call!(
            "POST",
            "/api/v1/agent/connections/runtime",
            Some(c),
            Some(foreign_report)
        )
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let mut foreign_discovery = c_discovery.clone();
    foreign_discovery["digest"] = b_connections["digest"].clone();
    foreign_discovery["sequence"] = 2.into();
    assert_eq!(
        call!(
            "POST",
            "/api/v1/agent/connections/discovery",
            Some(c),
            Some(foreign_discovery)
        )
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        call!(
            "POST",
            "/api/v1/agent/connection-identity",
            Some(c),
            Some(json!({"public_key":public_keys[0][0],"capabilities":capability}))
        )
        .0,
        StatusCode::CONFLICT
    );
    let flow = json!({"source_node_id":nodes[2][0],"destination_node_id":nodes[2][1],"destination_ip":null,"domain":null,"destination_country":null});
    let (_, foreign_preview) = call!(
        "POST",
        format!("{}/policy/preview", bases[1]),
        Some(&f.admin),
        Some(json!({"policy":b_policy["policy"],"flow":flow}))
    );
    assert_eq!(foreign_preview["valid"], false);
    let (_, foreign_flow) = call!(
        "POST",
        format!("{}/flows/preview", bases[1]),
        Some(&f.admin),
        Some(json!({"flow":flow}))
    );
    assert_eq!(foreign_flow["authorization"]["action"], "invalid");

    for suffix in [
        "overview",
        "nodes",
        "network",
        "tags",
        "access",
        "policy",
        "connections",
        "connections/runtime",
        "connections/discovery",
    ] {
        assert_eq!(
            call!(
                "GET",
                format!("{}/{suffix}", bases[0]),
                Some(&f.admin),
                None
            )
            .0,
            StatusCode::OK
        );
        assert_eq!(
            call!(
                "POST",
                "/api/v1/networks",
                Some(&f.admin),
                Some(json!({"name":format!("unrelated-{suffix}")}))
            )
            .0,
            StatusCode::OK
        );
        assert_eq!(
            call!("GET", "/api/v1/networks", Some(&f.admin), None).0,
            StatusCode::OK
        );
        assert_eq!(
            call!(
                "GET",
                "/api/v1/agents/configuration?network_id=not-C",
                Some(c),
                None
            )
            .1,
            c_config
        );
        assert_eq!(
            call!("GET", "/api/v1/agent/connections", Some(c), None).1,
            c_connections
        );
        assert_eq!(
            call!("GET", "/api/v1/agent/dataplane", Some(c), None).1,
            c_dataplane
        );
    }
    assert_eq!(
        call!(
            "POST",
            "/api/v1/agents/heartbeat",
            Some(c),
            Some(json!({"agent_version":"0.1.0"}))
        )
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call!(
            "POST",
            "/api/v1/agents/configuration/ack",
            Some(c),
            Some(json!({"revision":c_config["payload"]["revision"],"digest":c_config["digest"]}))
        )
        .0,
        StatusCode::OK
    );
    assert_eq!(call!("POST","/api/v1/agent/connections/ack",Some(c),Some(json!({"revision":c_connections["payload"]["revision"],"digest":c_connections["digest"]}))).0,StatusCode::OK);
    assert_eq!(call!("POST",format!("{}/nodes/country",bases[1]),Some(&f.admin),Some(json!({"node_id":nodes[1][0],"country":"CA","expected_revision":b_policy["revision"]}))).0,StatusCode::OK);
    assert_eq!(
        call!("GET", "/api/v1/agents/configuration", Some(c), None).1,
        c_config
    );
    let (_, runtime) = call!(
        "GET",
        format!("{}/connections/runtime", bases[2]),
        Some(&f.admin),
        None
    );
    assert_eq!(runtime["nodes"][0]["node_id"], nodes[2][0]);
    assert_eq!(runtime["nodes"][0]["digest"], c_connections["digest"]);
    assert_eq!(runtime["nodes"][0]["sequence"], 1);
    let (_, discovery) = call!(
        "GET",
        format!("{}/connections/discovery", bases[2]),
        Some(&f.admin),
        None
    );
    assert_eq!(discovery["network_id"], networks[2]);
    let own = discovery["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["node_id"] == nodes[2][0])
        .unwrap();
    assert_eq!(own["sequence"], 1);
    let (_, b_runtime) = call!(
        "GET",
        format!("{}/connections/runtime", bases[1]),
        Some(&f.admin),
        None
    );
    assert_eq!(b_runtime["nodes"], json!([]));
    let (_, b_discovery) = call!(
        "GET",
        format!("{}/connections/discovery", bases[1]),
        Some(&f.admin),
        None
    );
    assert!(
        b_discovery["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .all(|n| n["sequence"] == 0)
    );
    let (_, own_preview) = call!(
        "POST",
        format!("{}/flows/preview", bases[2]),
        Some(&f.admin),
        Some(json!({"flow":flow}))
    );
    assert_eq!(own_preview["authorization"]["action"], "allow");
    assert_eq!(
        own_preview["routing"]["path"],
        json!([nodes[2][0], nodes[2][1]])
    );
    assert_eq!(call!("GET", "/api/v1/agent/dns", Some(c), None).1, c_dns);
    let (_, b_code) = call!(
        "POST",
        format!("{}/device-authorizations", bases[1]),
        Some(&b_user),
        Some(
            json!({"redirect_uri":meshway_protocol::DEVICE_CALLBACK_URI,"code_challenge":meshway_protocol::s256(&"w".repeat(43)),"name":"B App","agent_version":"scope"})
        )
    );
    let binding = json!({"redirect_uri":meshway_protocol::DEVICE_CALLBACK_URI,"code_verifier":"w".repeat(43),"node_credential":local::credential().unwrap(),"name":"B App","agent_version":"scope"});
    let (status, app) = call!(
        "POST",
        "/api/v1/devices/exchange",
        b_code["code"].as_str(),
        Some(binding.clone())
    );
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        call!(
            "POST",
            "/api/v1/devices/exchange",
            b_code["code"].as_str(),
            Some(binding)
        )
        .1,
        app
    );
    assert_eq!(
        call!(
            "POST",
            format!("{}/nodes/revoke", bases[1]),
            Some(&f.admin),
            Some(json!({"node_id":app["node"]["id"]}))
        )
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call!("GET", format!("{}/nodes", bases[0]), Some(&f.admin), None).1,
        a_before
    );
    assert_eq!(
        call!("GET", "/api/v1/agents/configuration", Some(c), None).1,
        c_config
    );
    assert_eq!(
        call!("GET", "/api/v1/agent/dataplane", Some(c), None).1,
        c_dataplane
    );
    let (status, managed) = call!("GET", "/api/v1/agent/managed-dataplane", Some(c), None);
    assert_eq!(status, StatusCode::OK);
    assert_eq!(managed["dataplane"], c_dataplane);
    assert_eq!(managed["materials"], json!([]));
    assert_eq!(
        call!("GET", "/api/v1/agent/managed-connections", Some(c), None).0,
        StatusCode::OK
    );
    assert_eq!(
        call!(
            "GET",
            "/api/v1/agent/managed-dataplane",
            Some(&f.admin),
            None
        )
        .0,
        StatusCode::UNAUTHORIZED
    );
    let (status, public) = call!(
        "GET",
        format!("{}/managed-listeners", bases[0]),
        Some(&f.admin),
        None
    );
    assert_eq!(status, StatusCode::OK);
    assert_eq!(public["listeners"], json!([]));
    let intent = json!({"id":"optional","node_id":"","enabled":true,"transport":"native_udp","bind_address":"0.0.0.0","bind_port":18443,"advertised_endpoints":[],"vless":null});
    let mut wrong_scope = intent.clone();
    wrong_scope["node_id"] = nodes[2][0].clone().into();
    assert_eq!(
        call!(
            "POST",
            format!("{}/managed-listeners", bases[0]),
            Some(&f.admin),
            Some(json!({"expected_revision":public["revision"],"settings":wrong_scope}))
        )
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call!(
            "GET",
            format!("{}/managed-listeners", bases[0]),
            Some(c),
            None
        )
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call!(
            "POST",
            format!("{}/managed-enrollment-tokens", bases[0]),
            Some(&f.admin),
            Some(json!({"expires_in_seconds":600,"initial_listener":intent}))
        )
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call!(
            "POST",
            "/api/v1/agent/managed-transport-capabilities",
            Some(c),
            Some(json!({"vless_streams":[]}))
        )
        .0,
        StatusCode::OK
    );
    // V3 source activation and execution remain network/credential/epoch scoped.
    let source_path = format!("{}/policy-v3", bases[0]);
    let (status, source) = call!("GET", &source_path, Some(&f.admin), None);
    assert_eq!(status, StatusCode::OK);
    assert_eq!(source["source_mode"], "legacy_v2");
    let p = serde_json::to_value(meshway_policy::v3::NetworkPolicy::default_template()).unwrap();
    let write = json!({"expected_revision":source["revision"],"policy":p});
    assert_eq!(
        call!(
            "POST",
            format!("{source_path}/save"),
            Some(c),
            Some(write.clone())
        )
        .0,
        StatusCode::UNAUTHORIZED
    );
    let (status, saved) = call!(
        "POST",
        format!("{source_path}/save"),
        Some(&f.admin),
        Some(write.clone())
    );
    assert_eq!(status, StatusCode::OK);
    assert_eq!(saved["source_mode"], "mesh_v3");
    assert_eq!(
        call!(
            "POST",
            format!("{source_path}/save"),
            Some(&f.admin),
            Some(write)
        )
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        call!(
            "GET",
            format!("{}/policy-v3", bases[2]),
            Some(&f.admin),
            None
        )
        .1["source_mode"],
        "legacy_v2"
    );
    let a = &credentials[0][0];
    let instance = local::credential().unwrap();
    assert_eq!(
        call!(
            "POST",
            "/api/v1/agent/connections/runtime/begin",
            Some(a),
            Some(json!({"expected_epoch":0,"instance_id":instance}))
        )
        .0,
        StatusCode::OK
    );
    let runtime = json!({"epoch":1,"instance_id":instance,"capabilities":["ipv4_exact_path_v1"]});
    let (status, execution) = call!(
        "POST",
        "/api/v1/agents/mesh-execution",
        Some(a),
        Some(runtime.clone())
    );
    assert_eq!(status, StatusCode::OK);
    assert_eq!(execution["snapshot"]["payload"]["network_id"], networks[0]);
    assert_eq!(execution["snapshot"]["payload"]["node_id"], nodes[0][0]);
    assert_eq!(execution["snapshot"]["payload"]["peers"], json!([]));
    assert_eq!(
        call!(
            "POST",
            "/api/v1/agents/mesh-execution",
            Some(c),
            Some(runtime.clone())
        )
        .0,
        StatusCode::CONFLICT
    );
    let ack = json!({"runtime":runtime,"revision":execution["revision"],"digest":execution["snapshot"]["digest"]});
    assert_eq!(
        call!(
            "POST",
            "/api/v1/agents/mesh-execution/ack",
            Some(c),
            Some(ack.clone())
        )
        .0,
        StatusCode::CONFLICT
    );
    let (status, receipt) = call!(
        "POST",
        "/api/v1/agents/mesh-execution/ack",
        Some(a),
        Some(ack)
    );
    assert_eq!(status, StatusCode::OK);
    assert_eq!(receipt["saved"], true);
    let legacy = call!("GET", "/api/v1/agent/dataplane", Some(a), None).1;
    assert_eq!(legacy["payload"]["access_valid"], false);
    assert_eq!(legacy["payload"]["access_policy"]["grants"], json!([]));
    assert_eq!(
        call!("GET", format!("{}/policy", bases[2]), Some(&f.admin), None).1,
        c_policy
    );
    let api: Value = serde_json::from_str(&openapi_json().unwrap()).unwrap();
    let declared: BTreeSet<_> = api["paths"]
        .as_object()
        .unwrap()
        .iter()
        .filter(|(path, _)| !accounts::is_account_path(path))
        .flat_map(|(path, item)| {
            item.as_object()
                .unwrap()
                .keys()
                .map(move |method| (method.clone(), path.clone()))
        })
        .collect();
    assert_eq!(
        covered, declared,
        "new endpoint requires an explicit isolation matrix case"
    );
}
