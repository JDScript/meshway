//! Structured nft transactions; never interpolate the nft text language.
use serde_json::{Value, json};

use super::{
    ExitError,
    command::{Commands, Tool},
};
use meshway_model::exit::ExitUnavailableReason as Reason;

pub(super) fn probe_batch(name: &str) -> Value {
    // nft 1.0.9's JSON parser selects ct_templates by key and ignores `family`.
    // The explicit typed keys also work without packet protocol inference.
    json!({"nftables": [
        {"create": {"table": {"family": "inet", "name": name}}},
        {"add": {"set": {
            "family": "inet", "table": name, "name": "flows",
            "type": ["ipv4_addr", "ipv4_addr", "inet_proto", "inet_service", "inet_service"],
            "flags": ["timeout"], "timeout": 1, "size": 1
        }}},
        {"add": {"element": {"family": "inet", "table": name, "name": "flows", "elem": [
            {"elem": {"val": {"concat": ["192.0.2.1", "198.51.100.1", 17, 12345, 53]}, "timeout": 1}}
        ]}}},
        {"add": {"chain": {"family": "inet", "table": name, "name": "check"}}},
        {"add": {"rule": {"family": "inet", "table": name, "chain": "check", "expr": [
            {"match": {"op": "==", "left": {"ct": {"key": "direction"}}, "right": "original"}},
            {"match": {"op": "==", "left": {"concat": [
                {"ct": {"key": "ip saddr", "dir": "original"}},
                {"ct": {"key": "ip daddr", "dir": "original"}},
                {"ct": {"key": "protocol"}},
                {"ct": {"key": "proto-src", "dir": "original"}},
                {"ct": {"key": "proto-dst", "dir": "original"}}
            ]}, "right": "@flows"}},
            {"mangle": {"key": {"ct": {"key": "mark"}}, "value": {"|": [{"ct": {"key": "mark"}}, 1]}}},
            {"return": null}
        ]}}},
        {"delete": {"table": {"family": "inet", "name": name}}}
    ]})
}

pub(super) async fn submit(commands: &mut Commands, batch: &Value) -> Result<(), ExitError> {
    let bytes = serde_json::to_vec(batch).map_err(|_| Reason::NftUnavailable)?;
    let result = commands
        .run(Tool::Nft, vec!["-j".into(), "-f".into(), "-".into()], bytes)
        .await
        .map_err(|_| Reason::NftUnavailable)?;
    if !result.success {
        #[cfg(test)]
        eprintln!(
            "nft fixture rejection: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        return Err(Reason::NftUnavailable.into());
    }
    Ok(())
}

fn collision(tables: &[(String, String)], name: &str) -> Result<(), ExitError> {
    if tables
        .iter()
        .any(|(family, table)| family == "inet" && table == name)
    {
        return Err(Reason::OwnershipConflict.into());
    }
    Ok(())
}

pub(super) async fn verify_absent(commands: &mut Commands, name: &str) -> Result<(), ExitError> {
    let tables = super::super::tool::nft::tables(commands)
        .await
        .map_err(|_| Reason::NftUnavailable)?;
    collision(&tables, name)
}

#[cfg(test)]
mod tests;
