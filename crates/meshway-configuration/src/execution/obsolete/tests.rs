use super::*;
use serde_json::{Value, json};

const ID: &str = "11111111-1111-4111-8111-111111111111";

fn body() -> Value {
    let key = |algorithm| {
        json!({
            "id": ID, "generation": 1,
            "publicKey": { "algorithm": algorithm, "bytes": "11".repeat(32) }
        })
    };
    json!({
        "networkId": ID, "nodeId": ID, "revision": 1, "publication": ID,
        "dependencyDigest": "22".repeat(32), "enabled": true, "hostname": "node",
        "identity": { "type": "tag", "tags": ["tag:node"] },
        "addresses": { "ipv4": null, "ipv6": null },
        "selfKeys": {
            "controlKey": key("ed25519"), "dataplaneKey": key("x25519"),
            "capabilities": {}
        },
        "listeners": {}, "dialers": {}, "peers": {}, "permits": {},
        "diagnostics": {}, "decisions": {},
        "dns": {
            "settings": { "domain": "mesh.test" },
            "meshHosts": {}, "associations": [], "profile": "default"
        },
        "exitEnabled": false
    })
}

fn raw_digest(raw: &str) -> Digest {
    Digest::from_bytes(Sha256::digest(raw.as_bytes()).into())
}

fn validate(raw: &str) -> Result<ObsoleteSliceHeader, ObsoleteSliceError> {
    validate_obsolete_slice(&RawValue::from_string(raw.into()).unwrap(), raw_digest(raw))
}

fn reject(value: Value, expected: ObsoleteSliceError) {
    assert_eq!(validate(&value.to_string()).unwrap_err(), expected);
}

#[test]
fn historical_header_is_typed_but_context_checks_belong_to_callers() {
    let mut value = body();
    // The format validator does not bind network/node/revision or enforce key
    // algorithms by purpose; Server and Node retain those distinct checks.
    value["nodeId"] = json!("22222222-2222-4222-8222-222222222222");
    value["revision"] = json!(7);
    value["selfKeys"]["controlKey"]["publicKey"]["algorithm"] = json!("x25519");
    let header = validate(&value.to_string()).unwrap();
    assert_eq!(header.network_id.to_string(), ID);
    assert_eq!(
        header.node_id.to_string(),
        value["nodeId"].as_str().unwrap()
    );
    assert_eq!(header.revision.get(), 7);
    assert_eq!(header.publication.to_string(), ID);
    assert_eq!(header.dependency_digest, Digest::from_bytes([0x22; 32]));
    assert_eq!(
        header.self_keys.control_key.public_key.algorithm,
        meshway_model::KeyAlgorithm::X25519
    );
}

#[test]
fn historical_dial_endpoints_are_validated_without_becoming_current_candidates() {
    let mut value = body();
    value["meshAddressPools"] = json!({"ipv4": ["100.64.0.0/10"], "ipv6": []});
    value["localEgress"] = json!({});
    value["dns"]["egress"] = json!({"type": "noExit"});
    // demand-6 dialer options carried `priority`, which the current types reject.
    value["dialers"]["peer/udp"] = json!({
        "peer": ID, "listenerKey": "udp", "priority": 100,
        "endpoints": [{"host": "10.0.0.1", "port": 35000}],
        "parameters": {"type": "native_udp", "options": {"enabled": true, "priority": 100}}
    });
    validate(&value.to_string()).unwrap();
    value["dialers"]["peer/udp"]["parameters"] = json!([]);
    reject(value.clone(), ObsoleteSliceError::Map);
    value["dialers"]["peer/udp"]["parameters"] = json!({"type": "native_udp", "options": {}});
    assert!(
        serde_json::from_value::<super::super::DialCandidate>(value["dialers"]["peer/udp"].clone())
            .is_err()
    );
    value["dialers"]["peer/udp"]["endpoints"][0]["source"] = json!("configured");
    validate(&value.to_string()).unwrap();
    value["dialers"]["peer/udp"]["endpoints"][0]["source"] = json!("unknown");
    reject(value, ObsoleteSliceError::Map);
}

#[test]
fn digest_covers_original_bytes_not_canonical_values() {
    let value = body();
    let pretty = serde_json::to_string_pretty(&value).unwrap();
    validate(&pretty).unwrap();
    let raw = RawValue::from_string(pretty.clone()).unwrap();
    assert_eq!(
        validate_obsolete_slice(&raw, crate::digest(&value).unwrap()).unwrap_err(),
        ObsoleteSliceError::DigestMismatch
    );
    let reordered = pretty
        .replacen('{', "{\"hostname\":\"node\",", 1)
        .replace("  \"hostname\": \"node\",\n", "");
    validate(&reordered).unwrap();
    assert_ne!(raw_digest(&reordered), raw_digest(&pretty));
    let altered = RawValue::from_string(pretty.replace("true", "false")).unwrap();
    assert_eq!(
        validate_obsolete_slice(&altered, raw_digest(&pretty)).unwrap_err(),
        ObsoleteSliceError::DigestMismatch
    );
}

#[test]
fn field_set_is_exact_and_current_additions_are_rejected() {
    for field in body().as_object().unwrap().keys() {
        let mut value = body();
        value.as_object_mut().unwrap().remove(field);
        reject(value, ObsoleteSliceError::Fields);
    }
    for field in [
        "unknown",
        "meshAddressPools",
        "localEgress",
        "egressCapture",
        "exitRole",
    ] {
        let mut value = body();
        value[field] = json!({});
        reject(value, ObsoleteSliceError::Fields);
    }
    // Exit plans belong to demand-6 bodies only, which also carry capture fields.
    let mut value = body();
    value["egressCapture"] = json!({});
    value["exitRole"] = json!(null);
    reject(value, ObsoleteSliceError::Fields);
}

#[test]
fn demand6_exit_plans_are_typed_within_the_capture_shape() {
    let mut value = body();
    value["meshAddressPools"] = json!({"ipv4": ["100.64.0.0/10"], "ipv6": []});
    value["localEgress"] = json!({});
    value["dns"]["egress"] = json!({"type": "noExit"});
    value["egressCapture"] = json!({
        "generation": "33".repeat(32), "managedIpv4": [], "defaultCapture": false,
        "directIpv4": [], "unmanagedIpv4": [], "sinkIpv6": [], "unavailable": null
    });
    value["exitRole"] = json!(null);
    validate(&value.to_string()).unwrap();
    value["egressCapture"] = json!({"generation": "33".repeat(32)});
    reject(value.clone(), ObsoleteSliceError::Field);
    value.as_object_mut().unwrap().remove("egressCapture");
    reject(value, ObsoleteSliceError::Fields);
}

#[test]
fn duplicates_are_rejected_before_field_or_digest_checks_at_every_depth() {
    let raw = body().to_string();
    for (original, replacement) in [
        ("\"networkId\":", "\"networkId\":null,\"networkId\":"),
        ("\"enabled\":true", "\"enabled\":false,\"enabled\":true"),
        (
            "\"capabilities\":{}",
            "\"capabilities\":{\"x\":null,\"x\":null}",
        ),
        (
            "\"associations\":[]",
            "\"associations\":[{\"x\":1,\"x\":2}]",
        ),
        ("\"meshHosts\":{}", "\"meshHosts\":{\"x\":null,\"x\":null}"),
    ] {
        let changed = raw.replacen(original, replacement, 1);
        assert_ne!(changed, raw);
        let raw = RawValue::from_string(changed).unwrap();
        assert_eq!(
            validate_obsolete_slice(&raw, Digest::from_bytes([0; 32])).unwrap_err(),
            ObsoleteSliceError::Decode
        );
    }
}

#[test]
fn malformed_headers_and_non_object_bodies_are_rejected() {
    for value in [json!(null), json!([]), json!(true), json!("body")] {
        reject(value, ObsoleteSliceError::Decode);
    }
    for field in [
        "networkId",
        "nodeId",
        "revision",
        "publication",
        "dependencyDigest",
        "selfKeys",
    ] {
        let mut value = body();
        value[field] = json!("invalid");
        reject(value, ObsoleteSliceError::Decode);
    }
    let mut value = body();
    value["selfKeys"]["extra"] = json!(true);
    reject(value, ObsoleteSliceError::Decode);
}

#[test]
fn field_types_and_unknown_nested_fields_remain_strict() {
    for field in [
        "enabled",
        "hostname",
        "identity",
        "addresses",
        "exitEnabled",
    ] {
        let mut value = body();
        value[field] = json!(null);
        reject(value, ObsoleteSliceError::Field);
    }
    let mut value = body();
    value["enabled"] = json!("true");
    reject(value, ObsoleteSliceError::Field);
    for field in ["identity", "addresses"] {
        let mut value = body();
        value[field]["unknown"] = json!(true);
        reject(value, ObsoleteSliceError::Field);
    }
}

#[test]
fn maps_validate_shape_keys_and_values() {
    for field in [
        "listeners",
        "dialers",
        "peers",
        "permits",
        "diagnostics",
        "decisions",
    ] {
        for invalid in [json!([]), json!({ ID: null })] {
            let mut value = body();
            value[field] = invalid;
            reject(value, ObsoleteSliceError::Map);
        }
    }
    let mut value = body();
    value["decisions"] = json!({ "invalid-id": { "type": "direct" } });
    reject(value, ObsoleteSliceError::Map);
    let mut value = body();
    value["decisions"] = json!({ ID: { "type": "direct" } });
    validate(&value.to_string()).unwrap();
    // Distinct JSON strings can still identify the same typed map key.
    let mut value = body();
    value["decisions"] = json!({
        "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa": { "type": "direct" },
        "AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA": { "type": "direct" }
    });
    reject(value, ObsoleteSliceError::Map);
}

#[test]
fn dns_shape_and_nested_fields_keep_their_error_boundaries() {
    let mut value = body();
    value["dns"] = json!([]);
    reject(value, ObsoleteSliceError::Decode);
    for field in ["settings", "meshHosts", "associations", "profile"] {
        let mut value = body();
        value["dns"].as_object_mut().unwrap().remove(field);
        reject(value, ObsoleteSliceError::Dns);
        let mut value = body();
        value["dns"][field] = json!(null);
        reject(
            value,
            if field == "meshHosts" {
                ObsoleteSliceError::Map
            } else {
                ObsoleteSliceError::Field
            },
        );
        let mut value = body();
        value["dns"].as_object_mut().unwrap().remove(field);
        value["dns"]["unknown"] = json!(null);
        reject(value, ObsoleteSliceError::Field);
    }
    let mut value = body();
    value["dns"]["egress"] = json!({ "type": "noExit" });
    reject(value, ObsoleteSliceError::Dns);
    let mut value = body();
    value["dns"]["settings"]["unknown"] = json!(true);
    reject(value, ObsoleteSliceError::Field);
    let mut value = body();
    value["dns"]["meshHosts"] = json!({ "node": { "ipv4": "not-an-ip", "ipv6": null } });
    reject(value, ObsoleteSliceError::Map);
}
