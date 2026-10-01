//! Reproducible Rust client generation; kept out of ordinary controller builds.

use std::path::PathBuf;

use clap::Parser;
use openapi_to_rust::{CodeGenerator, GeneratorConfig, HttpClientConfig, SchemaAnalyzer};

#[path = "generate-rust-client/closed_objects.rs"]
mod closed_objects;
#[path = "generate-rust-client/required_nullable.rs"]
mod required_nullable;

#[derive(Parser)]
struct Args {
    #[arg(long)]
    input: PathBuf,
    #[arg(long)]
    output: PathBuf,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let document: serde_json::Value = serde_json::from_slice(&std::fs::read(&args.input)?)?;
    let operations = [
        "enrollAgent",
        "heartbeatAgent",
        "getAgentConfiguration",
        "acknowledgeConfiguration",
        "publishConnectionIdentity",
        "getAgentConnections",
        "acknowledgeAgentConnections",
        "beginConnectionRuntime",
        "getConnectionRuntimeEpoch",
        "reportConnectionRuntime",
    ];
    // Upstream has client-wide auth configuration. Derive it from the actual
    // selected operations, and fail if the contract needs richer semantics.
    let mut found = std::collections::BTreeSet::new();
    for item in document["paths"]
        .as_object()
        .ok_or("missing paths")?
        .values()
    {
        for operation in item.as_object().ok_or("invalid path item")?.values() {
            let Some(id) = operation["operationId"].as_str() else {
                continue;
            };
            if !operations.contains(&id) {
                continue;
            }
            if !found.insert(id.to_owned()) {
                return Err("duplicate agent operation".into());
            }
            let requirements = operation
                .get("security")
                .or_else(|| document.get("security"))
                .and_then(serde_json::Value::as_array)
                .ok_or("agent operation must declare security")?;
            if requirements.len() != 1 {
                return Err("agent operations require exactly one bearer scheme".into());
            }
            let requirement = requirements[0]
                .as_object()
                .ok_or("invalid security requirement")?;
            if requirement.len() != 1 {
                return Err("unsupported composite security requirement".into());
            }
            let (name, scopes) = requirement
                .iter()
                .next()
                .ok_or("missing bearer requirement")?;
            let scheme = &document["components"]["securitySchemes"][name];
            if scopes.as_array().is_none_or(|scopes| !scopes.is_empty())
                || scheme["type"] != "http"
                || scheme["scheme"] != "bearer"
            {
                return Err("unsupported agent authentication; extend generator first".into());
            }
        }
    }
    if found.len() != operations.len() {
        return Err("missing agent operation".into());
    }
    let mut analyzer = SchemaAnalyzer::new(document.clone())?;
    let mut analysis = analyzer.analyze()?;
    let generator = CodeGenerator::new(GeneratorConfig {
        spec_path: args.input,
        output_dir: args.output,
        module_name: "control_api".to_owned(),
        enable_async_client: true,
        enable_sse_client: false,
        tracing_enabled: false,
        client: Some(openapi_to_rust::config::ClientSection {
            operations: operations.into_iter().map(str::to_owned).collect(),
            prune_models: true,
        }),
        auth_config: Some(openapi_to_rust::http_config::AuthConfig::Bearer {
            header_name: "Authorization".to_owned(),
        }),
        http_client_config: Some(HttpClientConfig {
            base_url: Some("http://127.0.0.1:8420".to_owned()),
            timeout_seconds: Some(10),
            max_response_body_bytes: Some(2 * 1024 * 1024),
            default_headers: Default::default(),
        }),
        ..GeneratorConfig::default()
    });
    let mut result = generator.generate_all(&mut analysis)?;
    // Upstream 0.19 ignores timeout_seconds in client construction and follows
    // redirects. Never forward a proposed credential body to a redirect target.
    // This checked generation adapter must fail if upstream's template changes.
    let client = result
        .files
        .iter_mut()
        .find(|file| file.path == std::path::Path::new("client.rs"))
        .ok_or("missing generated client")?;
    let constructor = "let reqwest_client = reqwest::Client::new();";
    if client.content.matches(constructor).count() != 1 {
        return Err("upstream client constructor changed; review transport adapter".into());
    }
    client.content = client.content.replace(constructor, "let reqwest_client = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).timeout(std::time::Duration::from_secs(10)).build().expect(\"valid static HTTP client configuration\");");
    let types = result
        .files
        .iter_mut()
        .find(|file| file.path == std::path::Path::new("types.rs"))
        .ok_or("missing generated types")?;
    required_nullable::adapt(&document, &operations, &mut types.content)?;
    closed_objects::adapt(&document, &operations, &mut types.content)?;
    generator.write_files(&result)?;
    Ok(())
}
