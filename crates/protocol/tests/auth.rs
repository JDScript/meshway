use meshway_protocol::{
    AuthOptionsResponse, AuthSessionResponse, EmptyParams, ErrorResponse, LoginRequest,
    PasswordLengthUnit, PasswordRules, Problem, RegisterRequest, SessionIssuedResponse,
    SessionMetadata, UserView, WireSecret,
};

use serde::{Serialize, de::DeserializeOwned};

#[test]
fn structural_inputs_do_not_replace_account_semantics() {
    let request: RegisterRequest =
        serde_json::from_str(r#"{"email":"  User@bücher  ","password":"😀😀😀😀😀😀"}"#).unwrap();
    assert_eq!(request.email, "  User@bücher  ");
    assert_eq!(request.password.expose().chars().count(), 6);
    assert!(
        serde_json::from_str::<RegisterRequest>(
            r#"{"email":"a@b","password":"secret","admin":true}"#,
        )
        .is_err()
    );
    assert!(serde_json::from_str::<LoginRequest>(r#"{"email":"a@b","password":null}"#,).is_err());
    let secret = WireSecret::new("explicit wire secret".into());
    assert_eq!(
        serde_json::to_string(&secret).unwrap(),
        r#""explicit wire secret""#
    );
}

#[test]
fn problems_are_closed_and_request_id_is_required_nullable() {
    let unavailable: ErrorResponse = serde_json::from_str(
        r#"{"requestId":null,"status":503,"problem":{"code":"service_unavailable","params":{},"message":"Unavailable."}}"#,
    ).unwrap();
    assert!(unavailable.request_id.is_none());
    for invalid in [
        r#"{"status":503,"problem":{"code":"service_unavailable","params":{},"message":"Unavailable."}}"#,
        r#"{"requestId":null,"status":503,"problem":{"code":"service_unavailable","params":{"secret":"no"},"message":"Unavailable."}}"#,
        r#"{"requestId":null,"status":503,"problem":{"code":"service_unavailable","params":{},"message":"Unavailable."},"secret":"no"}"#,
    ] {
        assert!(serde_json::from_str::<ErrorResponse>(invalid).is_err());
    }
    for invalid in [
        r#"{"code":"invalid_session","params":{},"message":"Invalid.","secret":"no"}"#,
        r#"{"code":"unknown","params":{},"message":"Invalid."}"#,
        r#"{"code":"invalid_session","message":"Invalid."}"#,
        r#"{"code":"password_weak","params":{},"message":"Invalid."}"#,
        r#"{"code":"invalid_session","params":{}}"#,
    ] {
        assert!(serde_json::from_str::<Problem>(invalid).is_err());
    }
}

#[test]
fn wire_u16_fields_accept_exact_range_and_reject_other_numbers() {
    let rules = serde_json::json!({
        "minCodepoints": 12,
        "maxCodepoints": 128,
        "maxUtf8Bytes": 512,
        "lengthUnit": "unicodeCodepoints",
    });
    let error = serde_json::json!({
        "requestId": null,
        "status": 503,
        "problem": { "code": "service_unavailable", "params": {}, "message": "Unavailable." },
    });
    for (object, fields, is_rules) in [
        (
            &rules,
            &["minCodepoints", "maxCodepoints", "maxUtf8Bytes"][..],
            true,
        ),
        (&error, &["status"][..], false),
    ] {
        for field in fields {
            for (number, valid) in [
                ("0", true),
                ("65535", true),
                ("-1", false),
                ("65536", false),
                ("1.5", false),
                ("1e100", false),
            ] {
                let mut input = object.clone();
                input[field] = serde_json::from_str(number).unwrap();
                let text = input.to_string();
                let accepted = if is_rules {
                    serde_json::from_str::<PasswordRules>(&text).is_ok()
                } else {
                    serde_json::from_str::<ErrorResponse>(&text).is_ok()
                };
                assert_eq!(accepted, valid, "{field} accepted {number}: {accepted}");
            }
        }
    }
}

fn object_contract<T: Serialize + DeserializeOwned>(json: &str, fields: &[&str]) {
    let object: serde_json::Value = serde_json::from_str(json).unwrap();
    let decoded: T = serde_json::from_str(json).unwrap();
    assert_eq!(serde_json::to_value(decoded).unwrap(), object);
    let sequence =
        serde_json::Value::Array(fields.iter().map(|field| object[field].clone()).collect());
    assert!(serde_json::from_value::<T>(sequence.clone()).is_err());
    assert!(serde_json::from_str::<T>(&sequence.to_string()).is_err());
    let mut extra = object.clone();
    extra
        .as_object_mut()
        .unwrap()
        .insert("unexpected".into(), true.into());
    assert!(serde_json::from_value::<T>(extra).is_err());
    if let Some(field) = fields.first() {
        let duplicate = format!("{{\"{field}\":{},{}", object[field], &json[1..]);
        assert!(serde_json::from_str::<T>(&duplicate).is_err());
    }
}

#[test]
fn every_wire_object_rejects_sequence_unknown_and_duplicate_fields() {
    const ID: &str = "12345678-1234-4321-8765-123456789abc";
    let user = format!(r#"{{"id":"{ID}","email":"user@local"}}"#);
    let session = format!(
        r#"{{"id":"{ID}","createdAt":"2026-10-03T00:00:00Z","expiresAt":"2026-10-04T00:00:00Z"}}"#
    );
    let rules = r#"{"minCodepoints":12,"maxCodepoints":128,"maxUtf8Bytes":512,"lengthUnit":"unicodeCodepoints"}"#;
    let problem = r#"{"code":"invalid_session","params":{},"message":"Invalid session."}"#;
    let credentials = r#"{"email":"user@local","password":"user supplied password"}"#;
    object_contract::<RegisterRequest>(credentials, &["email", "password"]);
    object_contract::<LoginRequest>(credentials, &["email", "password"]);
    object_contract::<UserView>(&user, &["id", "email"]);
    object_contract::<SessionMetadata>(&session, &["id", "createdAt", "expiresAt"]);
    object_contract::<PasswordRules>(
        rules,
        &[
            "minCodepoints",
            "maxCodepoints",
            "maxUtf8Bytes",
            "lengthUnit",
        ],
    );
    object_contract::<EmptyParams>("{}", &[]);
    object_contract::<Problem>(problem, &["code", "params", "message"]);
    object_contract::<ErrorResponse>(
        &format!(r#"{{"requestId":null,"status":401,"problem":{problem}}}"#),
        &["requestId", "status", "problem"],
    );
    object_contract::<AuthOptionsResponse>(
        &format!(r#"{{"requestId":"{ID}","registrationEnabled":true,"passwordRules":{rules}}}"#),
        &["requestId", "registrationEnabled", "passwordRules"],
    );
    object_contract::<AuthSessionResponse>(
        &format!(r#"{{"requestId":"{ID}","user":{user},"session":{session}}}"#),
        &["requestId", "user", "session"],
    );
    object_contract::<SessionIssuedResponse>(
        &format!(
            r#"{{"requestId":"{ID}","user":{user},"session":{session},"token":"explicit token"}}"#
        ),
        &["requestId", "user", "session", "token"],
    );

    let nested_user_sequence =
        format!(r#"{{"requestId":"{ID}","user":["{ID}","user@local"],"session":{session}}}"#);
    assert!(serde_json::from_str::<AuthSessionResponse>(&nested_user_sequence).is_err());
    let nested_duplicate_user = format!(
        r#"{{"requestId":"{ID}","user":{{"id":"{ID}","email":"one@local","email":"two@local"}},"session":{session}}}"#
    );
    assert!(serde_json::from_str::<AuthSessionResponse>(&nested_duplicate_user).is_err());
    let nested_unknown_user = format!(
        r#"{{"requestId":"{ID}","user":{{"id":"{ID}","email":"one@local","admin":true}},"session":{session}}}"#
    );
    assert!(serde_json::from_str::<AuthSessionResponse>(&nested_unknown_user).is_err());
    assert!(
        serde_json::from_str::<Problem>(
            r#"{"code":"invalid_session","params":[],"message":"Invalid."}"#
        )
        .is_err()
    );
    assert!(serde_json::from_str::<PasswordLengthUnit>(r#"{"unicodeCodepoints":null}"#).is_err());
    assert!(serde_json::from_str::<PasswordLengthUnit>(r#""utf16""#).is_err());
}
