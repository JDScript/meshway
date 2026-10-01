use std::collections::BTreeSet;

use jsonc_parser::{CollectOptions, ParseOptions, ast::Value, parse_to_ast, parse_to_serde_value};
use serde::de::DeserializeOwned;

use crate::Error;

fn options() -> ParseOptions {
    ParseOptions {
        allow_comments: true,
        allow_loose_object_property_names: false,
        allow_trailing_commas: true,
        allow_missing_commas: false,
        allow_single_quoted_strings: false,
        allow_hexadecimal_numbers: false,
        allow_unary_plus_numbers: false,
    }
}

fn unique_keys(value: &Value<'_>) -> bool {
    match value {
        Value::Object(object) => {
            let mut keys = BTreeSet::new();
            object
                .properties
                .iter()
                .all(|property| keys.insert(property.name.as_str()) && unique_keys(&property.value))
        }
        Value::Array(array) => array.elements.iter().all(unique_keys),
        _ => true,
    }
}

pub(super) fn parse_source<T: DeserializeOwned>(source: &str) -> Result<T, Error> {
    let options = options();
    let parsed = parse_to_ast(source, &CollectOptions::default(), &options)
        .map_err(|_| Error::Unprocessable("source_jsonc is not valid strict JSONC"))?;
    let value = parsed.value.as_ref().ok_or(Error::Unprocessable(
        "source_jsonc must contain one JSON value",
    ))?;
    if !unique_keys(value) {
        return Err(Error::Unprocessable(
            "source_jsonc contains a duplicate object key",
        ));
    }
    parse_to_serde_value(source, &options)
        .map_err(|_| Error::Unprocessable("source_jsonc does not match the policy schema"))
}
