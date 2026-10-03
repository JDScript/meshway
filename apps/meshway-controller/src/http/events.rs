//! SSE declaration shared with the SDK emitter. A/B bind the generic stream
//! union to their root-exported concrete UserEvent and NodeEvent contracts.
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
