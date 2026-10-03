use meshway_model::{
    ActivityId, Digest, InstanceId, JoinLinkId, KeyId, MembershipId, MembershipRole, NetworkId,
    NodeId, PublicationId, RequestId, Revision, Sequence, SessionId, UserId, Version,
};
use serde_json::{Value, json};
use utoipa::{OpenApi, PartialSchema, ToSchema};

fn schema<T: PartialSchema>() -> Value {
    serde_json::to_value(T::schema()).unwrap()
}

#[test]
fn every_id_exports_the_full_canonical_nonzero_constraint() {
    let expected_pattern = "^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$";
    for exported in [
        schema::<UserId>(),
        schema::<NetworkId>(),
        schema::<NodeId>(),
        schema::<MembershipId>(),
        schema::<SessionId>(),
        schema::<RequestId>(),
        schema::<PublicationId>(),
        schema::<InstanceId>(),
        schema::<KeyId>(),
        schema::<ActivityId>(),
        schema::<JoinLinkId>(),
    ] {
        assert_eq!(exported["type"], "string");
        assert_eq!(exported["format"], "uuid");
        assert_eq!(exported["minLength"], 36);
        assert_eq!(exported["maxLength"], 36);
        assert_eq!(exported["pattern"], expected_pattern);
    }
}

#[test]
fn number_schemas_preserve_zero_rules_and_exact_safe_integer_ceiling() {
    for (exported, minimum) in [
        (schema::<Revision>(), 1),
        (schema::<Version>(), 1),
        (schema::<Sequence>(), 0),
    ] {
        assert_eq!(exported["type"], "integer");
        assert_eq!(exported["minimum"], json!(minimum));
        assert_eq!(exported["maximum"], json!(9_007_199_254_740_991_u64));
        assert!(exported.get("exclusiveMinimum").is_none());
        assert!(exported.get("exclusiveMaximum").is_none());
    }
}

#[test]
fn digest_schema_describes_hex_not_a_byte_array() {
    let exported = schema::<Digest>();
    assert_eq!(exported["type"], "string");
    assert_eq!(exported["minLength"], 64);
    assert_eq!(exported["maxLength"], 64);
    assert_eq!(exported["pattern"], "^[0-9a-f]{64}$");
}

#[test]
fn membership_roles_have_the_same_closed_wire_and_schema_vocabulary() {
    assert_eq!(
        schema::<MembershipRole>()["enum"],
        json!(["owner", "admin", "member"])
    );
    for (role, text) in [
        (MembershipRole::Owner, "owner"),
        (MembershipRole::Admin, "admin"),
        (MembershipRole::Member, "member"),
    ] {
        let encoded = serde_json::to_string(&role).unwrap();
        assert_eq!(encoded, format!("\"{text}\""));
        assert_eq!(
            serde_json::from_str::<MembershipRole>(&encoded).unwrap(),
            role
        );
    }
    for invalid in [
        "\"Owner\"",
        "\"administrator\"",
        "\" member\"",
        "\"\"",
        "null",
        "1",
    ] {
        assert!(serde_json::from_str::<MembershipRole>(invalid).is_err());
    }
}

#[test]
fn consumer_schema_keeps_separate_named_types_and_transitive_constraints() {
    #[derive(ToSchema)]
    #[allow(dead_code)]
    struct References {
        user: UserId,
        node: NodeId,
        revision: Revision,
        version: Version,
        sequence: Sequence,
        digest: Digest,
    }

    #[derive(OpenApi)]
    #[openapi(components(schemas(References)))]
    struct Api;

    let document = serde_json::to_value(Api::openapi()).unwrap();
    let components = &document["components"]["schemas"];
    for (property, name, exported) in [
        ("user", "UserId", schema::<UserId>()),
        ("node", "NodeId", schema::<NodeId>()),
        ("revision", "Revision", schema::<Revision>()),
        ("version", "Version", schema::<Version>()),
        ("sequence", "Sequence", schema::<Sequence>()),
        ("digest", "Digest", schema::<Digest>()),
    ] {
        assert_eq!(components[name], exported);
        assert_eq!(
            components["References"]["properties"][property]["$ref"],
            format!("#/components/schemas/{name}")
        );
    }
}
