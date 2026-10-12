//! End-to-end pipeline through the real binary: convert, inspect, match,
//! index (sequence increments), verify, and rejection of tampering.

use serde_json::Value;
use std::{path::Path, process::Command};

fn run(args: &[&str]) -> (bool, Value) {
    let out = Command::new(env!("CARGO_BIN_EXE_meshway-rules"))
        .args(args)
        .env("RUST_LOG", "error")
        .output()
        .expect("binary runs");
    let stdout = String::from_utf8(out.stdout).unwrap();
    (
        out.status.success(),
        serde_json::from_str(&stdout).unwrap_or(Value::Null),
    )
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

#[test]
fn publish_pipeline() {
    let tmp = tempfile::tempdir().unwrap();
    let dist = tmp.path().join("dist");
    std::fs::create_dir_all(dist.join("artifacts")).unwrap();
    std::fs::create_dir_all(dist.join("sets/meta-geosite")).unwrap();
    let source = tmp.path().join("openai.list");
    std::fs::write(&source, "+.openai.com\nchatgpt.com\n*.bad.example\n").unwrap();
    let artifact = tmp.path().join("openai.mwrs");

    let (ok, record) = run(&[
        "convert",
        "--format",
        "mihomo-domain-text",
        s(&source),
        "-o",
        s(&artifact),
    ]);
    assert!(ok, "{record}");
    assert_eq!(record["report"]["summary"]["unconverted"]["wildcard"], 1);
    assert_eq!(record["counts"]["suffix"], 1);
    let digest = record["digest"].as_str().unwrap().to_owned();

    let (ok, info) = run(&["inspect", s(&artifact)]);
    assert!(ok);
    assert_eq!(info["digest"], digest.as_str());
    assert_eq!(info["entries"], 2);
    assert_eq!(info["sections"].as_array().unwrap().len(), 4);

    let (ok, hit) = run(&["match", s(&artifact), "API.openai.com."]);
    assert!(ok);
    assert_eq!(hit["match"]["kind"], "suffix");
    assert_eq!(hit["member"], "openai.com");
    let (_, miss) = run(&["match", s(&artifact), "192.0.2.1"]);
    assert_eq!(miss["match"], false);

    // Pipeline layout: artifact by digest, set record with its upstream.
    let hex = digest.strip_prefix("sha256:").unwrap();
    std::fs::copy(&artifact, dist.join(format!("artifacts/sha256-{hex}.mwrs"))).unwrap();
    let set = serde_json::json!({
        "upstream": {
            "repo": "MetaCubeX/meta-rules-dat",
            "ref": "meta",
            "commit": "83c8208bdfae92b1caaa29745f51f233b6f1847e",
            "path": "geo/geosite/openai.list"
        },
        "conversion": record,
    });
    std::fs::write(dist.join("sets/meta-geosite/openai.json"), set.to_string()).unwrap();
    let key = tmp.path().join("key.hex");
    std::fs::write(&key, format!("{}\n", "3a".repeat(32))).unwrap();
    let index = dist.join("index.json");

    let (ok, first) = run(&["index", "--sign-key", s(&key), s(&dist), "-o", s(&index)]);
    assert!(ok, "{first}");
    assert_eq!(first["sequence"], 1);
    let (_, second) = run(&["index", "--sign-key", s(&key), s(&dist), "-o", s(&index)]);
    assert_eq!(second["sequence"], 2);
    let key_id = second["keyId"].as_str().unwrap().to_owned();

    let (ok, verified) = run(&["verify", "--public-key", &key_id, s(&index), s(&dist)]);
    assert!(ok, "{verified}");
    assert_eq!(verified["sets"], 1);
    let other = format!("ed25519:{}", "d7".repeat(32));
    assert!(!run(&["verify", "--public-key", &other, s(&index), s(&dist)]).0);

    // A replaced artifact under the same name fails verification.
    let path = dist.join(format!("artifacts/sha256-{hex}.mwrs"));
    let mut bytes = std::fs::read(&path).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    std::fs::write(&path, bytes).unwrap();
    assert!(!run(&["verify", "--public-key", &key_id, s(&index), s(&dist)]).0);
}
