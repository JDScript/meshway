//! Span edits preserve comments and spelling outside changed values. JSONC
//! trailing commas allow deleting a final property without moving comments.
use crate::{
    ConfigurationError, ConfigurationOperation, NetworkConfigurationInput, apply_operations,
    parse_jsonc,
};
use jsonc_parser::{
    ast,
    common::Ranged,
    tokens::{Token, TokenAndRange},
};
use serde_json::Value;

pub fn edit_source(
    source: &str,
    ops: &[ConfigurationOperation],
) -> Result<(String, NetworkConfigurationInput), ConfigurationError> {
    let before = parse_jsonc(source)?;
    let mut after = before.clone();
    apply_operations(&mut after, ops)?;
    let result = rewrite_source(source, &before, &after)?;
    Ok((result, after))
}

/// Rewrites a typed system change against the exact source baseline. Callers
/// must separately retain the complete normalized value and stable bindings.
pub fn rewrite_source(
    source: &str,
    before: &NetworkConfigurationInput,
    after: &NetworkConfigurationInput,
) -> Result<String, ConfigurationError> {
    if &parse_jsonc(source)? != before {
        return Err(ConfigurationError::Serialization);
    }
    let a = serde_json::to_value(before).map_err(|_| ConfigurationError::Serialization)?;
    let b = serde_json::to_value(after).map_err(|_| ConfigurationError::Serialization)?;
    let ast = jsonc_parser::parse_to_ast(
        source,
        &jsonc_parser::CollectOptions {
            tokens: true,
            ..Default::default()
        },
        &Default::default(),
    )
    .map_err(|_| ConfigurationError::Parse)?;
    let root = ast.value.ok_or(ConfigurationError::Parse)?;
    let mut edits = Vec::new();
    collect(
        &root,
        &a,
        &b,
        ast.tokens.as_deref().ok_or(ConfigurationError::Parse)?,
        &mut edits,
    )?;
    edits.sort_by_key(|(start, end, _)| (*start, *end));
    let mut result = source.to_owned();
    for (start, end, text) in edits.into_iter().rev() {
        result.replace_range(start..end, &text);
    }
    // Do not return text unless it still represents exactly the typed edit.
    if &parse_jsonc(&result)? != after {
        return Err(ConfigurationError::Serialization);
    }
    Ok(result)
}
type Edit = (usize, usize, String);
fn collect(
    node: &ast::Value<'_>,
    a: &Value,
    b: &Value,
    tokens: &[TokenAndRange<'_>],
    edits: &mut Vec<Edit>,
) -> Result<(), ConfigurationError> {
    if a == b {
        return Ok(());
    }
    let (ast::Value::Object(object), Value::Object(a), Value::Object(b)) = (node, a, b) else {
        let range = node.range();
        edits.push((
            range.start,
            range.end,
            serde_json::to_string(b).map_err(|_| ConfigurationError::Serialization)?,
        ));
        return Ok(());
    };
    let mut retained_last = None;
    for (index, prop) in object.properties.iter().enumerate() {
        let name = prop.name.as_str();
        if let Some(target) = b.get(name) {
            retained_last = Some(index);
            if let Some(before) = a.get(name) {
                collect(&prop.value, before, target, tokens, edits)?;
            }
        } else {
            edits.push((prop.range.start, prop.range.end, String::new()));
            let end = object
                .properties
                .get(index + 1)
                .map_or(object.range.end - 1, |p| p.range.start);
            if let Some(comma) = tokens.iter().find(|t| {
                t.range.start >= prop.range.end
                    && t.range.end <= end
                    && matches!(t.token, Token::Comma)
            }) {
                edits.push((comma.range.start, comma.range.end, String::new()));
            }
        }
    }
    let additions = b
        .iter()
        .filter(|(key, value)| {
            a.get(*key) != Some(*value)
                && !object
                    .properties
                    .iter()
                    .any(|p| p.name.as_str() == key.as_str())
        })
        .map(|(key, value)| {
            Ok(format!(
                "{}:{}",
                serde_json::to_string(key).map_err(|_| ConfigurationError::Serialization)?,
                serde_json::to_string(value).map_err(|_| ConfigurationError::Serialization)?
            ))
        })
        .collect::<Result<Vec<_>, ConfigurationError>>()?;
    if !additions.is_empty() {
        let comma = retained_last.is_some_and(|i| {
            let prop = &object.properties[i];
            let end = object
                .properties
                .get(i + 1)
                .map_or(object.range.end - 1, |p| p.range.start);
            !tokens.iter().any(|t| {
                t.range.start >= prop.range.end
                    && t.range.end <= end
                    && matches!(t.token, Token::Comma)
            })
        });
        edits.push((
            object.range.end - 1,
            object.range.end - 1,
            format!(
                "\n{}{}\n",
                if comma { "," } else { "" },
                additions.join(",")
            ),
        ));
    }
    Ok(())
}
