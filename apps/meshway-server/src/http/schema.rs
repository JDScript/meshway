//! Reject conflicting public schema names before Utoipa merges erase their origin.
use std::collections::BTreeMap;
use utoipa::openapi::{OpenApi, RefOr, schema::Schema};

#[derive(Default)]
pub(super) struct Registry(BTreeMap<String, serde_json::Value>);

impl Registry {
    pub(super) fn add<'a>(
        &mut self,
        schemas: impl IntoIterator<Item = (&'a String, &'a RefOr<Schema>)>,
    ) {
        for (name, schema) in schemas {
            let mut value = serde_json::to_value(schema).expect("schema serialization");
            normalize(&mut value);
            if let Some(previous) = self.0.get(name) {
                assert_eq!(previous, &value, "conflicting OpenAPI component: {name}");
            } else {
                self.0.insert(name.clone(), value);
            }
        }
    }

    pub(super) fn spec(&mut self, spec: &OpenApi) {
        if let Some(components) = &spec.components {
            self.add(&components.schemas);
        }
    }
}

// Internally tagged Serde variants are closed in the public emitter. Utoipa
// sometimes omits that annotation until SSE registration; compare that common
// form while preserving every field, type, bound, reference and required key.
fn normalize(value: &mut serde_json::Value) {
    if let Some(variants) = value
        .get_mut("oneOf")
        .and_then(serde_json::Value::as_array_mut)
    {
        for variant in variants {
            if let Some(object) = variant.as_object_mut()
                && object
                    .get("properties")
                    .and_then(serde_json::Value::as_object)
                    .is_some_and(|p| !p.is_empty())
            {
                object
                    .entry("additionalProperties")
                    .or_insert(serde_json::Value::Bool(false));
            }
            normalize(variant);
        }
    }
}

pub(super) fn merge(spec: &mut OpenApi, other: OpenApi) {
    let mut registry = Registry::default();
    registry.spec(spec);
    registry.spec(&other);
    spec.merge(other);
}

pub(super) fn extend(spec: &mut OpenApi, schemas: Vec<(String, RefOr<Schema>)>) {
    let mut registry = Registry::default();
    registry.spec(spec);
    registry.add(schemas.iter().map(|(name, schema)| (name, schema)));
    spec.components
        .get_or_insert_with(Default::default)
        .schemas
        .extend(schemas);
}

#[cfg(test)]
mod tests {
    use super::*;
    use utoipa::openapi::schema::{ObjectBuilder, Type};

    #[test]
    #[should_panic(expected = "conflicting OpenAPI component: Duplicate")]
    fn conflicting_schema_names_fail_before_map_insertion() {
        let schemas = [
            (
                "Duplicate".to_owned(),
                ObjectBuilder::new().schema_type(Type::String).into(),
            ),
            (
                "Duplicate".to_owned(),
                ObjectBuilder::new().schema_type(Type::Boolean).into(),
            ),
        ];
        Registry::default().add(schemas.iter().map(|(n, s)| (n, s)));
    }

    #[test]
    fn execution_failures_are_published_and_ip_lookups_are_not() {
        let spec = serde_json::to_value(crate::http::openapi()).unwrap();
        let schemas = &spec["components"]["schemas"];
        assert!(
            schemas["UnavailableReason"]["enum"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!("unauthorized"))
        );
        // IP datasets serve automatic routing only; no HTTP type carries a lookup.
        for name in ["IpInfo", "GeoIpData", "LookupUnavailableReason"] {
            assert!(schemas.get(name).is_none(), "{name}");
        }
    }
}
