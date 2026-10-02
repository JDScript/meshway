//! Standalone JSON Schema for the independent policy editor contract.
use meshway_policy::*;
use serde_json::{Value, json};
use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(components(schemas(
    NetworkPolicy,
    ConnectionRule,
    ConnectionAction,
    Grant,
    MeshRule,
    ExitRule,
    ExitMatch,
    ExitTarget,
    RoutingProfile,
    ProfileFallback,
    AutomaticRouting,
    ProfileAssignment
)))]
struct Schema;

fn rewrite_refs(value: &mut Value) {
    match value {
        Value::Object(object) => {
            if let Some(Value::String(reference)) = object.get_mut("$ref")
                && let Some(name) = reference.strip_prefix("#/components/schemas/")
            {
                *reference = format!("#/$defs/{name}");
            }
            for value in object.values_mut() {
                rewrite_refs(value);
            }
        }
        Value::Array(values) => {
            for value in values {
                rewrite_refs(value);
            }
        }
        _ => {}
    }
}

fn main() {
    let schemas = Schema::openapi()
        .components
        .expect("derived components")
        .schemas;
    let mut schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$ref": "#/$defs/NetworkPolicy",
        "$defs": schemas,
    });
    rewrite_refs(&mut schema);
    println!(
        "{}",
        serde_json::to_string_pretty(&schema).expect("schema JSON")
    );
}
