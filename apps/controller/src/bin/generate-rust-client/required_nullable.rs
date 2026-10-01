//! Checked workaround for upstream required-nullable fields becoming ordinary
//! serde Options. The live OpenAPI, restricted to selected operations' reachable
//! components, determines every field to adapt. No DTO or field names are copied.

use std::collections::BTreeSet;

use serde_json::Value;

const DESERIALIZER: &str = r#"
// Generated from required nullable OpenAPI properties. deserialize_with makes
// serde require presence; Option accepts explicit null. Never skip None on output.
fn deserialize_required_nullable<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}
"#;

pub fn adapt(document: &Value, operation_ids: &[&str], source: &mut String) -> Result<(), String> {
    let reachable = reachable_components(document, operation_ids)?;
    let mut adapted = false;
    for name in reachable {
        let schema = document["components"]["schemas"]
            .get(&name)
            .ok_or("missing referenced schema")?;
        if !contains_nullable(schema) {
            continue;
        }
        adapt_nullable_component(source, &name, schema)?;
        adapted = true;
    }
    if adapted {
        if source.contains("fn deserialize_required_nullable") {
            return Err("required-nullable helper collides with generated code".into());
        }
        source.push_str(DESERIALIZER);
    }
    Ok(())
}

pub(super) fn reachable_components(
    document: &Value,
    operation_ids: &[&str],
) -> Result<BTreeSet<String>, String> {
    let mut pending = Vec::new();
    for item in document["paths"]
        .as_object()
        .ok_or("missing paths")?
        .values()
    {
        for operation in item.as_object().ok_or("invalid path item")?.values() {
            if operation["operationId"]
                .as_str()
                .is_some_and(|id| operation_ids.contains(&id))
            {
                references(operation, &mut pending)?;
            }
        }
    }
    let mut visited = BTreeSet::new();
    while let Some(name) = pending.pop() {
        if !visited.insert(name.clone()) {
            continue;
        }
        let schema = document["components"]["schemas"]
            .get(&name)
            .ok_or("missing referenced schema")?;
        references(schema, &mut pending)?;
    }
    Ok(visited)
}

fn adapt_nullable_component(source: &mut String, name: &str, schema: &Value) -> Result<(), String> {
    // Nullable aliases, compositions, inline objects, array items and
    // optional nullable fields need their own presence-aware mappings.
    if schema["type"] != "object" || schema.get("nullable").is_some() {
        return Err(format!(
            "unsupported nullable component {name}; extend required-nullable adapter"
        ));
    }
    let properties = schema["properties"]
        .as_object()
        .ok_or("nullable component has no properties")?;
    let required = schema["required"]
        .as_array()
        .ok_or("nullable component has no required fields")?;
    for (field, property) in properties {
        if !contains_nullable(property) {
            continue;
        }
        let types = property["type"]
            .as_array()
            .ok_or("only direct scalar T|null properties are supported")?;
        if types.len() != 2
            || types.iter().filter(|ty| **ty == "null").count() != 1
            || !types.iter().any(|ty| {
                matches!(
                    ty.as_str(),
                    Some("string" | "integer" | "number" | "boolean")
                )
            })
            || !required.iter().any(|value| value == field)
        {
            return Err(format!(
                "unsupported nullable property {name}.{field}; required scalar T|null expected"
            ));
        }
        for key in property
            .as_object()
            .ok_or("invalid nullable property")?
            .keys()
        {
            if ![
                "type",
                "format",
                "description",
                "title",
                "minLength",
                "maxLength",
                "pattern",
                "minimum",
                "maximum",
            ]
            .contains(&key.as_str())
            {
                return Err(format!("unsupported nullable property keyword {key}"));
            }
        }
        adapt_field(source, name, field)?;
    }
    // Do not silently miss nullability introduced outside direct properties.
    for (key, value) in schema.as_object().ok_or("invalid nullable component")? {
        if key != "properties" && contains_nullable(value) {
            return Err(format!("unsupported nullable shape in {name}.{key}"));
        }
    }
    Ok(())
}

fn references(value: &Value, pending: &mut Vec<String>) -> Result<(), String> {
    match value {
        Value::Object(object) => {
            if let Some(reference) = object.get("$ref") {
                let name = reference
                    .as_str()
                    .and_then(|value| value.strip_prefix("#/components/schemas/"))
                    .ok_or("required-nullable adapter only supports component schema references")?;
                pending.push(name.to_owned());
            }
            for value in object.values() {
                references(value, pending)?;
            }
        }
        Value::Array(values) => {
            for value in values {
                references(value, pending)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn contains_nullable(value: &Value) -> bool {
    match value {
        Value::Object(object) => {
            object.contains_key("nullable")
                || object.get("type").is_some_and(|ty| {
                    ty == "null"
                        || ty
                            .as_array()
                            .is_some_and(|types| types.iter().any(|ty| ty == "null"))
                })
                || object
                    .get("enum")
                    .and_then(Value::as_array)
                    .is_some_and(|values| values.iter().any(Value::is_null))
                || object.get("const").is_some_and(Value::is_null)
                || object.values().any(contains_nullable)
        }
        Value::Array(values) => values.iter().any(contains_nullable),
        _ => false,
    }
}

fn adapt_field(source: &mut String, component: &str, field: &str) -> Result<(), String> {
    // Fail rather than guessing if upstream renames a model/field, changes its
    // layout, or introduces serde defaults/skip rules. This pinned template has
    // simple Rust identifiers and one-line Option fields without serde attrs.
    if !component.bytes().all(|byte| byte.is_ascii_alphanumeric())
        || !field
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    {
        return Err(
            "required-nullable adapter needs simple component and field identifiers".into(),
        );
    }
    let declaration = format!("pub struct {component} {{\n");
    if source.matches(&declaration).count() != 1 {
        return Err(format!(
            "expected one generated struct for nullable component {component}"
        ));
    }
    let declaration_start = source
        .find(&declaration)
        .ok_or("missing generated struct")?;
    for line in source[..declaration_start].lines().rev() {
        if line.trim() == "}" {
            break;
        }
        if line.trim().starts_with("#[serde") {
            return Err(format!(
                "unexpected generated container serde attributes for {component}"
            ));
        }
    }
    let start = declaration_start + declaration.len();
    let end = start
        + source[start..]
            .find("\n}")
            .ok_or("unclosed generated struct")?;
    let body = &source[start..end];
    let prefix = format!("pub {field}: Option<");
    let mut offset = start;
    let mut attributes = Vec::new();
    let mut target = None;
    for line in body.split_inclusive('\n') {
        let trimmed = line.trim();
        if trimmed.starts_with("#[") {
            attributes.push(trimmed);
        }
        if trimmed.starts_with("pub ") {
            if trimmed.starts_with(&prefix) {
                if target.is_some() || !trimmed.ends_with(">,") || !attributes.is_empty() {
                    return Err(format!(
                        "unexpected generated serde/field shape for {component}.{field}"
                    ));
                }
                target = Some(offset);
            }
            attributes.clear();
        }
        offset += line.len();
    }
    let target =
        target.ok_or_else(|| format!("missing generated Option field {component}.{field}"))?;
    source.insert_str(
        target,
        "    #[serde(deserialize_with = \"deserialize_required_nullable\")]\n",
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn document(property: Value, required: bool) -> Value {
        json!({
            "paths": {"/test": {"post": {"operationId": "selected", "responses": {"200": {"content": {"application/json": {"schema": {"$ref": "#/components/schemas/Example"}}}}}}}},
            "components": {"schemas": {
                "Example": {"type": "object", "properties": {"deleted_at": property}, "required": if required { vec!["deleted_at"] } else { vec![] }},
                "Unused": {"type": ["object", "null"]}
            }}
        })
    }

    #[test]
    fn required_nullable_adaptation_follows_schema_and_operation_scope() {
        let mut source =
            "pub struct Example {\n    pub deleted_at: Option<String>,\n}\n".to_owned();
        adapt(
            &document(json!({"type": ["null", "string"]}), true),
            &["selected"],
            &mut source,
        )
        .unwrap();
        assert!(source.contains("#[serde(deserialize_with = \"deserialize_required_nullable\")]\n    pub deleted_at: Option<String>"));
        assert!(!source.contains("skip_serializing_if"));
        assert!(!source.contains("serde(default"));
        assert_eq!(
            source.matches("fn deserialize_required_nullable").count(),
            1
        );
    }

    #[test]
    fn unsupported_nullable_shapes_and_changed_upstream_templates_fail_closed() {
        let source = "pub struct Example {\n    pub deleted_at: Option<String>,\n}\n";
        for (property, required) in [
            (json!({"type": ["string", "null"]}), false),
            (json!({"anyOf": [{"type":"string"}, {"type":"null"}]}), true),
            (
                json!({"type":"object", "properties": {"inner": {"type":["string","null"]}}}),
                true,
            ),
            (json!({"type":["string","null"], "default":null}), true),
        ] {
            assert!(
                adapt(
                    &document(property, required),
                    &["selected"],
                    &mut source.to_owned()
                )
                .is_err()
            );
        }
        for changed in [
            source.replace("Example", "Renamed"),
            source.replace("deleted_at", "renamed"),
            format!("#[serde(default)]\n{source}"),
            source.replace(
                "    pub deleted_at",
                "    #[serde(default)]\n    pub deleted_at",
            ),
            source.replace(
                "    pub deleted_at",
                "    #[serde(skip_serializing_if = \"Option::is_none\")]\n    pub deleted_at",
            ),
        ] {
            assert!(
                adapt(
                    &document(json!({"type":["string","null"]}), true),
                    &["selected"],
                    &mut changed.clone()
                )
                .is_err()
            );
        }
    }
}
