use meshway_protocol::ipc::{Command, EnrollmentKey, VERSION};
use serde_json::json;

#[test]
fn identity_commands_are_typed_optional_and_secret_debug_is_redacted() {
    assert_eq!(VERSION, 7);
    let old_up: Command =
        serde_json::from_value(json!({"type":"up","expectedRevision":1})).unwrap();
    assert!(matches!(
        old_up,
        Command::Up {
            enrollment_key: None,
            ..
        }
    ));
    let command = Command::Up {
        expected_revision: 1.try_into().unwrap(),
        enrollment_key: Some(EnrollmentKey::new("private-fixture-value".into()).unwrap()),
        reenroll: false,
    };
    assert!(!format!("{command:?}").contains("private-fixture-value"));
    let value = serde_json::to_value(command).unwrap();
    assert_eq!(value["enrollmentKey"], "private-fixture-value");
    assert!(value.get("reenroll").is_none(), "omitted unless requested");
    let reenroll: Command =
        serde_json::from_value(json!({"type":"up","expectedRevision":1,"reenroll":true})).unwrap();
    assert!(matches!(
        reenroll,
        Command::Up {
            reenroll: true,
            enrollment_key: None,
            ..
        }
    ));
    assert!(
        serde_json::from_value::<Command>(json!({"type":"rotateKey","expectedRevision":1})).is_ok()
    );
    assert!(matches!(
        serde_json::from_value::<Command>(json!({"type":"logout","expectedRevision":1})).unwrap(),
        Command::Logout { .. }
    ));
    assert!(serde_json::from_value::<Command>(json!({"type":"logout"})).is_err());
    assert!(matches!(
        serde_json::from_value::<Command>(json!({"type":"netcheck"})).unwrap(),
        Command::Netcheck
    ));
    for key in [
        json!(""),
        json!({"secret":"private"}),
        json!(["private"]),
        json!("x".repeat(4097)),
    ] {
        assert!(
            serde_json::from_value::<Command>(
                json!({"type":"up","expectedRevision":1,"enrollmentKey":key})
            )
            .is_err()
        );
    }
    assert!(
        serde_json::from_value::<Command>(
            json!({"type":"rotateKey","expectedRevision":1,"token":"private"})
        )
        .is_err()
    );
}
