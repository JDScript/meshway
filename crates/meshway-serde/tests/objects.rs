use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::json;
use utoipa::{OpenApi, PartialSchema, ToSchema};

#[derive(Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Nested {
    value: String,
}

fn nullable<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Nested>, D::Error> {
    Deserialize::deserialize(d)
}

meshway_serde::object! {
    #[derive(Debug, PartialEq)]
    pub struct Envelope {
        pub rows: Vec<Nested>,
        pub map: BTreeMap<String, Nested>,
        #[serde(deserialize_with = "nullable")]
        pub optional_value: Option<Nested> [required = true],
    }
}

meshway_serde::tagged! {
    #[derive(Debug, PartialEq)]
    pub enum Edit {
        Keep,
        Replace {
            #[serde(rename = "nextValue", deserialize_with = "nullable")]
            next_value: Option<Nested> [required = true],
        },
    }
}

meshway_serde::tagged! {
    tag = "delivery";
    #[derive(Debug, PartialEq)]
    pub enum Delivery {
        Missing {},
        Available { value: String },
    }
}

#[derive(OpenApi)]
#[openapi(components(schemas(Envelope, Edit, Delivery)))]
struct Api;

meshway_serde::object! { pub struct DeclaredChild { pub value: String } }
meshway_serde::object! { pub struct DeclaredParent {
    pub child: DeclaredChild,
    pub children: Vec<DeclaredChild>,
} }

#[test]
fn nested_declared_objects_reuse_the_strict_boundary_for_text_and_value_inputs() {
    let valid = r#"{"child":{"value":"one"},"children":[{"value":"two"}]}"#;
    assert!(serde_json::from_str::<DeclaredParent>(valid).is_ok());
    assert!(serde_json::from_value::<DeclaredParent>(serde_json::from_str(valid).unwrap()).is_ok());
    for text in [
        r#"{"child":["one"],"children":[]}"#,
        r#"{"child":{"value":"one"},"children":[["two"]]}"#,
        r#"{"child":{"value":"one","value":"two"},"children":[]}"#,
        r#"{"child":{"value":"one"},"children":[{"value":"one","value":"two"}]}"#,
    ] {
        assert!(serde_json::from_str::<DeclaredParent>(text).is_err());
    }
}

#[test]
fn nested_objects_maps_and_arrays_keep_one_strict_decoding_policy() {
    let valid =
        r#"{"rows":[{"value":"one"}],"map":{"first":{"value":"two"}},"optionalValue":null}"#;
    let decoded: Envelope = serde_json::from_str(valid).unwrap();
    assert_eq!(
        serde_json::to_value(&decoded).unwrap(),
        serde_json::from_str::<serde_json::Value>(valid).unwrap()
    );
    for text in [
        r#"[[{"value":"one"}],{},null]"#,
        r#"{"rows":[["one"]],"map":{},"optionalValue":null}"#,
        r#"{"rows":[],"map":{"first":["two"]},"optionalValue":null}"#,
        r#"{"rows":[],"map":{},"optionalValue":["three"]}"#,
        r#"{"rows":[],"map":{},"optionalValue":{"value":"one","value":"two"}}"#,
        r#"{"rows":[],"map":{"first":{"value":"one"},"\u0066irst":{"value":"two"}},"optionalValue":null}"#,
        r#"{"rows":[],"map":{}}"#,
        r#"{"rows":[],"map":{},"optionalValue":null,"extra":true}"#,
    ] {
        assert!(
            serde_json::from_str::<Envelope>(text).is_err(),
            "accepted {text}"
        );
    }
}

#[test]
fn tagged_units_custom_tags_and_nullable_fields_match_closed_schemas() {
    assert_eq!(
        serde_json::from_str::<Edit>(r#"{"type":"keep"}"#).unwrap(),
        Edit::Keep
    );
    assert_eq!(
        serde_json::from_str::<Edit>(r#"{"type":"replace","nextValue":null}"#).unwrap(),
        Edit::Replace { next_value: None }
    );
    for text in [
        r#""keep""#,
        r#"{"type":"keep","nextValue":null}"#,
        r#"{"type":"replace"}"#,
        r#"{"type":"replace","nextValue":["positional"]}"#,
        r#"{"type":"keep","\u0074ype":"keep"}"#,
    ] {
        assert!(
            serde_json::from_str::<Edit>(text).is_err(),
            "accepted {text}"
        );
    }
    let delivery: Delivery =
        serde_json::from_str(r#"{"delivery":"available","value":"ok"}"#).unwrap();
    assert_eq!(
        serde_json::to_value(delivery).unwrap(),
        json!({"delivery":"available","value":"ok"})
    );
    assert!(
        serde_json::from_str::<Delivery>(r#"{"delivery":"missing","value":"not allowed"}"#)
            .is_err()
    );

    let schema = serde_json::to_value(Api::openapi()).unwrap();
    let components = &schema["components"]["schemas"];
    assert!(components.get("Nested").is_some());
    assert_eq!(components["Envelope"]["additionalProperties"], false);
    for branch in components["Edit"]["oneOf"].as_array().unwrap() {
        assert_eq!(branch["additionalProperties"], false);
        assert!(
            branch["required"]
                .as_array()
                .unwrap()
                .contains(&json!("type"))
        );
        if branch["properties"]["type"]["enum"][0] == "replace" {
            assert!(
                branch["required"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("nextValue"))
            );
        }
    }
    let custom = serde_json::to_value(Delivery::schema()).unwrap();
    assert!(custom["oneOf"].as_array().unwrap().iter().all(|branch| {
        branch["required"]
            .as_array()
            .unwrap()
            .contains(&json!("delivery"))
    }));
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct CaseFolded(String);
impl<'de> Deserialize<'de> for CaseFolded {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(Self(String::deserialize(d)?.to_lowercase()))
    }
}
#[derive(Deserialize)]
struct NormalizedKeys {
    #[serde(deserialize_with = "meshway_serde::unique_map")]
    entries: BTreeMap<CaseFolded, Nested>,
}

#[test]
fn typed_map_keys_cannot_collide_after_normalization() {
    let value: NormalizedKeys =
        serde_json::from_str(r#"{"entries":{"A":{"value":"one"},"B":{"value":"two"}}}"#).unwrap();
    assert_eq!(value.entries.len(), 2);
    for text in [
        r#"{"entries":{"A":{"value":"one"},"a":{"value":"two"}}}"#,
        r#"{"entries":{"a":{"value":"one"},"a":{"value":"two"}}}"#,
        r#"{"entries":{"a":["positional"]}}"#,
    ] {
        assert!(
            serde_json::from_str::<NormalizedKeys>(text).is_err(),
            "accepted {text}"
        );
    }
}

meshway_serde::string_enum! {
    #[derive(Debug, Default, PartialEq, Serialize, ToSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum StringMode {
        #[default]
        NotReady,
        #[serde(rename = "ready.now")]
        Ready,
    }
}
meshway_serde::tagged! {
    tag = "kind";
    #[derive(Debug, PartialEq)]
    pub enum ModeEvent {
        Changed = "mode.changed" { mode: StringMode },
    }
}

#[test]
fn string_enums_keep_renames_and_reject_object_aliases_after_tagged_buffering() {
    for wire in ["not_ready", "ready.now"] {
        let mode: StringMode = serde_json::from_value(json!(wire)).unwrap();
        assert_eq!(serde_json::to_value(mode).unwrap(), wire);
        let event: ModeEvent =
            serde_json::from_value(json!({"kind":"mode.changed","mode":wire})).unwrap();
        assert_eq!(
            serde_json::to_value(event).unwrap(),
            json!({"kind":"mode.changed","mode":wire})
        );
        assert!(serde_json::from_value::<StringMode>(json!({(wire):null})).is_err());
        assert!(
            serde_json::from_value::<ModeEvent>(
                json!({"kind":"mode.changed","mode":{(wire):null}})
            )
            .is_err()
        );
    }
    let schema = serde_json::to_value(StringMode::schema()).unwrap();
    assert_eq!(schema["enum"], json!(["not_ready", "ready.now"]));
    let event_schema = serde_json::to_value(ModeEvent::schema()).unwrap();
    assert_eq!(
        event_schema["oneOf"][0]["properties"]["kind"]["enum"],
        json!(["mode.changed"])
    );
    assert_eq!(event_schema["oneOf"][0]["additionalProperties"], false);
}

#[derive(Serialize, Deserialize)]
pub enum ExternalPayload {
    Newtype(Nested),
    Tuple(Nested, Nested),
    Struct { nested: Nested },
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalEnvelope {
    #[serde(deserialize_with = "meshway_serde::deserialize_strict")]
    pub payload: ExternalPayload,
}

#[test]
fn strict_decoding_traverses_external_enum_access_and_variant_payloads() {
    for wire in [
        r#"{"payload":{"Newtype":{"value":"ok"}}}"#,
        r#"{"payload":{"Tuple":[{"value":"one"},{"value":"two"}]}}"#,
        r#"{"payload":{"Struct":{"nested":{"value":"ok"}}}}"#,
    ] {
        assert!(serde_json::from_str::<ExternalEnvelope>(wire).is_ok());
    }
    for wire in [
        r#"{"payload":{"Newtype":["positional"]}}"#,
        r#"{"payload":{"Tuple":[["positional"],{"value":"ok"}]}}"#,
        r#"{"payload":{"Struct":[{"value":"ok"}]}}"#,
        r#"{"payload":{"Struct":{"nested":["positional"]}}}"#,
        r#"{"payload":{"Newtype":{"value":"one","value":"two"}}}"#,
    ] {
        assert!(
            serde_json::from_str::<ExternalEnvelope>(wire).is_err(),
            "{wire}"
        );
    }
}
