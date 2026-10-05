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

#[test]
fn original_fraction_and_leap_seconds_cannot_be_normalized_into_milliseconds() {
    for time in [
        "1970-01-01T00:00:00.0000000001Z",
        "1970-01-01T00:00:00.0010000000000001Z",
        "1970-01-01T00:00:00.0001Z",
        "2016-12-31T23:59:60Z",
        "2016-12-31t23:59:60.001z",
        "1969-12-31T23:59:59.999Z",
        "9999-12-31T23:59:59-01:00",
        "2026-10-04 00:00:00Z",
        "2026-10-04T00:00:00.Z",
        "2026-10-04T00:00:00Z\n",
        "+2026-10-04T00:00:00Z",
    ] {
        for input in [
            serde_json::json!({"at":time,"optional":null}),
            serde_json::json!({"at":"1970-01-01T00:00:00Z","optional":time}),
        ] {
            assert!(serde_json::from_value::<Times>(input).is_err(), "{time}");
        }
    }
    for time in [
        "1970-01-01T00:00:00.001Z",
        "1970-01-01t00:00:00.0010000000000000000z",
        "1970-01-01T01:00:00.001+01:00",
        "1969-12-31T23:00:00.001-01:00",
    ] {
        let input = serde_json::json!({"at":time,"optional":time});
        let parsed: Times = serde_json::from_value(input).unwrap();
        assert_eq!(
            parsed,
            Times {
                at: 1,
                optional: Some(1)
            }
        );
    }
}

#[test]
fn timestamp_marker_publishes_one_named_string_contract() {
    use meshway_model::timestamp::Timestamp;
    use utoipa::{PartialSchema, ToSchema};
    let schema = serde_json::to_value(Timestamp::schema()).unwrap();
    assert_eq!(Timestamp::name(), "Timestamp");
    assert_eq!(schema["type"], "string");
    assert_eq!(schema["format"], "meshway-timestamp-ms");
}
