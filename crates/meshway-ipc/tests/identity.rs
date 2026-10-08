use meshway_ipc::{Command, EnrollmentKey, VERSION};
use serde_json::json;

#[test]
fn v5_identity_commands_are_typed_optional_and_secret_debug_is_redacted() {
    assert_eq!(VERSION, 5);
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
    };
    assert!(!format!("{command:?}").contains("private-fixture-value"));
    let value = serde_json::to_value(command).unwrap();
    assert_eq!(value["enrollmentKey"], "private-fixture-value");
    assert!(
        serde_json::from_value::<Command>(json!({"type":"rotateKey","expectedRevision":1})).is_ok()
    );
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
