//! Honor OpenAPI additionalProperties=false before serde can discard unknown data.
//! Runs after required-nullable adaptation and fails on unsupported template shapes.

use serde_json::Value;

pub fn adapt(document: &Value, operations: &[&str], source: &mut String) -> Result<(), String> {
    for name in super::required_nullable::reachable_components(document, operations)? {
        let schema = &document["components"]["schemas"][&name];
        if schema["type"] != "object" {
            continue;
        }
        if schema["additionalProperties"] != false || !schema["properties"].is_object() {
            return Err(format!(
                "unsupported open object {name}; extend closed-object adapter"
            ));
        }
        if name.is_empty() || !name.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
            return Err("closed-object adapter requires simple component identifiers".into());
        }
        let declaration = format!("pub struct {name} {{\n");
        if source.matches(&declaration).count() != 1 {
            return Err(format!(
                "expected one generated struct for closed component {name}"
            ));
        }
        let start = source
            .find(&declaration)
            .ok_or("missing generated closed object")?;
        for line in source[..start].lines().rev() {
            if line.trim() == "}" {
                break;
            }
            if line.trim().starts_with("#[serde") {
                return Err(format!(
                    "unexpected generated container serde attributes for {name}"
                ));
            }
        }
        let body_start = start + declaration.len();
        let body_end = body_start
            + source[body_start..]
                .find("\n}")
                .ok_or("unclosed generated object")?;
        if source[body_start..body_end].contains("serde(flatten") {
            return Err(format!(
                "unsupported flattened fields in closed component {name}"
            ));
        }
        source.insert_str(start, "#[serde(deny_unknown_fields)]\n");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn document() -> Value {
        json!({
            "paths": {"/test": {"get": {"operationId": "selected", "responses": {"200": {"content": {"application/json": {"schema": {"$ref": "#/components/schemas/Envelope"}}}}}}}},
            "components": {"schemas": {
                "Envelope": {"type": "object", "additionalProperties": false, "properties": {"payload": {"$ref": "#/components/schemas/Payload"}}, "required": ["payload"]},
                "Payload": {"type": "object", "additionalProperties": false, "properties": {"key": {"type": ["string", "null"]}}, "required": ["key"]},
                "Unused": {"type": "object", "additionalProperties": true, "properties": {}}
            }}
        })
    }

    const SOURCE: &str = "#[derive(Deserialize)]\npub struct Envelope {\n    pub payload: Payload,\n}\n#[derive(Deserialize)]\npub struct Payload {\n    pub key: Option<String>,\n}\n";

    #[test]
    fn reachable_closed_objects_keep_required_nullable_rules_and_reject_extras() {
        let doc = document();
        let mut source = SOURCE.to_owned();
        super::super::required_nullable::adapt(&doc, &["selected"], &mut source).unwrap();
        adapt(&doc, &["selected"], &mut source).unwrap();
        assert_eq!(source.matches("#[serde(deny_unknown_fields)]").count(), 2);
        assert!(source.contains("#[serde(deserialize_with = \"deserialize_required_nullable\")]"));
        assert!(!source.contains("Unused"));
    }

    #[test]
    fn open_or_renamed_or_flattened_templates_fail_generation() {
        let mut doc = document();
        doc["components"]["schemas"]["Payload"]["additionalProperties"] = json!(true);
        assert!(adapt(&doc, &["selected"], &mut SOURCE.to_owned()).is_err());
        let doc = document();
        for changed in [
            SOURCE.replace("pub struct Payload", "pub struct Renamed"),
            SOURCE.replace(
                "    pub payload:",
                "    #[serde(flatten)]\n    pub payload:",
            ),
            SOURCE.replace(
                "pub struct Payload",
                "#[serde(default)]\npub struct Payload",
            ),
            format!("{SOURCE}\npub struct Payload {{\n}}\n"),
        ] {
            assert!(adapt(&doc, &["selected"], &mut changed.clone()).is_err());
        }
    }
}
