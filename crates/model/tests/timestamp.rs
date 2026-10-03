use serde::{Deserialize, Serialize};

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Times {
    #[serde(with = "meshway_model::timestamp")]
    at: u64,
    #[serde(with = "meshway_model::timestamp::optional")]
    optional: Option<u64>,
}

#[test]
fn timestamps_keep_exact_milliseconds_and_explicit_null_on_the_wire() {
    let value = Times {
        at: 1,
        optional: None,
    };
    assert_eq!(
        serde_json::to_string(&value).unwrap(),
        r#"{"at":"1970-01-01T00:00:00.001Z","optional":null}"#
    );
    assert_eq!(
        serde_json::from_str::<Times>(r#"{"at":"1970-01-01T01:00:00.001+01:00","optional":null}"#)
            .unwrap(),
        value
    );
    for text in [
        r#"{"at":1,"optional":null}"#,
        r#"{"at":"1969-12-31T23:59:59Z","optional":null}"#,
        r#"{"at":"1970-01-01T00:00:00.000001Z","optional":null}"#,
        r#"{"at":"1970-01-01T00:00:00Z"}"#,
    ] {
        assert!(serde_json::from_str::<Times>(text).is_err());
    }
    assert!(
        serde_json::to_string(&Times {
            at: u64::MAX,
            optional: None
        })
        .is_err()
    );
    assert!(
        serde_json::to_string(&Times {
            at: 0,
            optional: Some(u64::MAX)
        })
        .is_err()
    );
}
