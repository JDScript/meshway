//! SSE declaration shared with the SDK emitter. The normal Router binds the
//! generic stream to concrete root-exported UserEvent and NodeEvent contracts.
use utoipa::{ToSchema, openapi::OpenApi};

/// `T` must be a concrete root-exported meshway_protocol event type. The
/// generated operation declares the entire response as string bytes; only
/// x-meshway-sse describes the per-frame JSON data object.
pub fn declare_sse<T: ToSchema>(spec: &mut OpenApi, path: &str) -> Result<(), &'static str> {
    let mut schemas = vec![(T::name().into_owned(), T::schema())];
    T::schemas(&mut schemas);
    spec.components
        .get_or_insert_with(Default::default)
        .schemas
        .extend(schemas);
    let operation = spec
        .paths
        .paths
        .get_mut(path)
        .and_then(|item| item.get.as_mut())
        .ok_or("missing actual SSE GET operation")?;
    operation.extensions.get_or_insert_with(Default::default).insert("x-meshway-sse".into(),serde_json::json!({"format":"json-data","eventSchema":{"$ref":format!("#/components/schemas/{}",T::name())}}));
    Ok(())
}

/// Utoipa omits `deny_unknown_fields` from inline internally tagged variants.
/// The State model's unions all deny unknown fields; preserve that contract in
/// the exported schema. Typed map schemas and nullable/reference arms are kept.
pub(super) fn close_state_unions(spec: &mut OpenApi) {
    use utoipa::openapi::{
        RefOr,
        schema::{AdditionalProperties, Schema},
    };
    fn close(schema: &mut RefOr<Schema>) {
        if let RefOr::T(Schema::OneOf(union)) = schema {
            for branch in &mut union.items {
                if let RefOr::T(Schema::Object(object)) = branch
                    && !object.properties.is_empty()
                    && object.additional_properties.is_none()
                {
                    object.additional_properties =
                        Some(Box::new(AdditionalProperties::FreeForm(false)));
                }
            }
        }
    }
    if let Some(components) = &mut spec.components {
        for schema in components.schemas.values_mut() {
            close(schema);
        }
    }
}
