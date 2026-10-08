//! Request and response examples of the exported contract.
//!
//! Examples live in `packages/protocol/examples/<tag>.json`, keyed by
//! operation ID: `{"login": {"request": {...}, "responses": {"200": {...}}}}`.
//! Error responses without an explicit example carry the Server's own sample
//! for the first problem code the response declares.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;
use utoipa::openapi::{OpenApi, RefOr, path::Operation};

use super::error::problem_samples;

const FILES: &[(&str, &str)] = &[
    (
        "account",
        include_str!("../../../../packages/protocol/examples/account.json"),
    ),
    (
        "auth",
        include_str!("../../../../packages/protocol/examples/auth.json"),
    ),
    (
        "configuration",
        include_str!("../../../../packages/protocol/examples/configuration.json"),
    ),
    (
        "connectivity",
        include_str!("../../../../packages/protocol/examples/connectivity.json"),
    ),
    (
        "enrollment",
        include_str!("../../../../packages/protocol/examples/enrollment.json"),
    ),
    (
        "execution",
        include_str!("../../../../packages/protocol/examples/execution.json"),
    ),
    (
        "history",
        include_str!("../../../../packages/protocol/examples/history.json"),
    ),
    (
        "members",
        include_str!("../../../../packages/protocol/examples/members.json"),
    ),
    (
        "networks",
        include_str!("../../../../packages/protocol/examples/networks.json"),
    ),
    (
        "node",
        include_str!("../../../../packages/protocol/examples/node.json"),
    ),
    (
        "nodes",
        include_str!("../../../../packages/protocol/examples/nodes.json"),
    ),
    (
        "observations",
        include_str!("../../../../packages/protocol/examples/observations.json"),
    ),
    (
        "resources",
        include_str!("../../../../packages/protocol/examples/resources.json"),
    ),
    (
        "state",
        include_str!("../../../../packages/protocol/examples/state.json"),
    ),
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    #[serde(default)]
    request: Option<Value>,
    #[serde(default)]
    responses: BTreeMap<String, Value>,
}

pub(super) fn attach(spec: &mut OpenApi) {
    let mut entries: BTreeMap<String, Entry> = BTreeMap::new();
    for (file, text) in FILES {
        let parsed: BTreeMap<String, Entry> =
            serde_json::from_str(text).unwrap_or_else(|e| panic!("examples/{file}.json: {e}"));
        for (operation, entry) in parsed {
            assert!(
                entries.insert(operation.clone(), entry).is_none(),
                "examples: {operation} is defined in more than one file"
            );
        }
    }
    let errors = problem_samples();
    for item in spec.paths.paths.values_mut() {
        for operation in [
            &mut item.get,
            &mut item.post,
            &mut item.put,
            &mut item.delete,
            &mut item.patch,
        ]
        .into_iter()
        .flatten()
        {
            let id = operation.operation_id.clone().expect("operation ID");
            let entry = entries.remove(&id).unwrap_or(Entry {
                request: None,
                responses: BTreeMap::new(),
            });
            attach_operation(operation, &id, entry, &errors);
        }
    }
    assert!(
        entries.is_empty(),
        "examples for unknown operations: {:?}",
        entries.keys().collect::<Vec<_>>()
    );
}

fn attach_operation(
    operation: &mut Operation,
    id: &str,
    mut entry: Entry,
    errors: &[super::error::ProblemSample],
) {
    if let Some(example) = entry.request.take() {
        let content = operation
            .request_body
            .as_mut()
            .and_then(|body| match body {
                RefOr::T(body) => body.content.get_mut("application/json"),
                RefOr::Ref(_) => None,
            })
            .and_then(|content| match content {
                RefOr::T(content) => Some(content),
                RefOr::Ref(_) => None,
            })
            .unwrap_or_else(|| panic!("{id}: request example without a JSON request body"));
        content.example = Some(example);
    }
    for (status, response) in operation.responses.responses.iter_mut() {
        let RefOr::T(response) = response else {
            continue;
        };
        let Some(RefOr::T(content)) = response.content.get_mut("application/json") else {
            continue;
        };
        if let Some(example) = entry.responses.remove(status) {
            content.example = Some(example);
            continue;
        }
        let is_error = matches!(
            &content.schema,
            Some(RefOr::Ref(reference))
                if reference.ref_location == "#/components/schemas/ErrorResponse"
        );
        if !is_error {
            continue;
        }
        let code = response
            .description
            .split(',')
            .next()
            .map(str::trim)
            .filter(|code| !code.is_empty())
            .unwrap_or_else(|| panic!("{id} {status}: error response without problem codes"));
        let status: u16 = status.parse().expect("numeric status");
        let sample = errors
            .iter()
            .find(|sample| sample.status == status && sample.code == code)
            .unwrap_or_else(|| panic!("{id} {status}: no sample for {code}"));
        content.example = Some(sample.body.clone());
    }
    assert!(
        entry.responses.is_empty(),
        "{id}: examples for responses the operation does not declare with a JSON body: {:?}",
        entry.responses.keys().collect::<Vec<_>>()
    );
}
