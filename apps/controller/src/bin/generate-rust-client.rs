//! Reproducible Rust client generation; kept out of ordinary controller builds.

use std::path::PathBuf;

use clap::Parser;
use openapi_to_rust::{CodeGenerator, GeneratorConfig, HttpClientConfig, SchemaAnalyzer};

#[derive(Parser)]
struct Args {
    #[arg(long)]
    input: PathBuf,
    #[arg(long)]
    output: PathBuf,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let mut analyzer = SchemaAnalyzer::new(serde_json::from_slice(&std::fs::read(&args.input)?)?)?;
    let mut analysis = analyzer.analyze()?;
    let generator = CodeGenerator::new(GeneratorConfig {
        spec_path: args.input,
        output_dir: args.output,
        module_name: "control_api".to_owned(),
        enable_async_client: true,
        enable_sse_client: false,
        tracing_enabled: false,
        client: Some(openapi_to_rust::config::ClientSection {
            operations: vec!["reportAgent".to_owned()],
            prune_models: true,
        }),
        http_client_config: Some(HttpClientConfig {
            base_url: Some("http://127.0.0.1:8420".to_owned()),
            timeout_seconds: Some(10),
            max_response_body_bytes: Some(2 * 1024 * 1024),
            default_headers: Default::default(),
        }),
        ..GeneratorConfig::default()
    });
    let result = generator.generate_all(&mut analysis)?;
    generator.write_files(&result)?;
    Ok(())
}
