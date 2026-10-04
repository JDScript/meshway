use meshway_protocol::{
    CreateNetworkRequest, NetworkListResponse, NetworkMembersResponse, NetworkPageQuery,
};
use serde_json::json;

#[test]
fn network_lists_retain_required_nullable_cursor_and_reject_invalid_roles() {
    let network = json!({
        "id": "12345678-1234-4234-8234-123456789abc",
        "name": "My network",
        "createdAt": "2026-10-03T12:00:00Z",
        "currentRevision": 1,
        "membershipId": "23456789-2345-4345-8345-23456789abcd",
        "role": "owner",
        "actions": {"rename": true, "editConfiguration": true, "manageMembers": true, "joinLinkRoles": ["owner", "admin", "member"]}
    });
    let valid = json!({
        "requestId": "34567890-3456-4456-8456-34567890abcd",
        "items": [network],
        "total": 1,
        "nextCursor": null,
        "asOf": "2026-10-03T12:00:00Z"
    });
    assert!(serde_json::from_value::<NetworkListResponse>(valid.clone()).is_ok());
    let mut positional = valid.clone();
    positional["items"][0]["actions"] = json!([true, true, true]);
    assert!(serde_json::from_value::<NetworkListResponse>(positional).is_err());
    for role in [
        json!({"owner":null}),
        json!("instance_admin"),
        json!(["owner"]),
    ] {
        let mut value = valid.clone();
        value["items"][0]["role"] = role;
        assert!(serde_json::from_value::<NetworkListResponse>(value).is_err());
    }
    let mut missing = valid;
    missing.as_object_mut().unwrap().remove("nextCursor");
    assert!(serde_json::from_value::<NetworkListResponse>(missing).is_err());
    assert!(serde_json::from_str::<NetworkMembersResponse>("[]").is_err());
}

#[test]
fn network_creation_has_no_caller_controlled_creator_or_membership_role() {
    let valid = json!({
        "requestId": "34567890-3456-4456-8456-34567890abcd",
        "name": "A network"
    });
    assert!(serde_json::from_value::<CreateNetworkRequest>(valid.clone()).is_ok());
    for (field, value) in [
        ("userId", json!("forged")),
        ("role", json!("owner")),
        ("networkId", json!("forged")),
    ] {
        let mut candidate = valid.clone();
        candidate[field] = value;
        assert!(serde_json::from_value::<CreateNetworkRequest>(candidate).is_err());
    }
    assert!(
        serde_json::from_str::<CreateNetworkRequest>(
            r#"{"requestId":"34567890-3456-4456-8456-34567890abcd","name":"first","name":"second"}"#
        )
        .is_err()
    );
}

#[test]
fn paging_parameters_are_optional_nonnullable_uri_scalars() {
    use utoipa::IntoParams;
    let parameters = serde_json::to_value(NetworkPageQuery::into_params(|| None)).unwrap();
    let parameters = parameters.as_array().unwrap();
    assert_eq!(parameters.len(), 2);
    let limit = parameters.iter().find(|p| p["name"] == "limit").unwrap();
    assert_eq!(limit["in"], "query");
    assert_eq!(limit["required"], false);
    assert_eq!(limit["schema"]["type"], "integer");
    assert_eq!(limit["schema"]["minimum"], 1);
    assert_eq!(limit["schema"]["maximum"], 200);
    let cursor = parameters.iter().find(|p| p["name"] == "cursor").unwrap();
    assert_eq!(cursor["required"], false);
    assert_eq!(cursor["schema"]["type"], "string");
    assert_eq!(cursor["schema"]["maxLength"], 4096);
}
