//! Structured nft transactions; never interpolate the nft text language.
use serde::Deserialize;
use serde_json::{Value, json};

use super::{
    ExitError,
    command::{Commands, Tool},
};
use meshway_model::exit::ExitUnavailableReason as Reason;

pub(super) fn probe_batch(name: &str) -> Value {
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
                {"ct": {"key": "src", "family": "ip", "dir": "original"}},
                {"ct": {"key": "dst", "family": "ip", "dir": "original"}},
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
        return Err(Reason::NftUnavailable.into());
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Tables {
    nftables: Vec<Entry>,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum Entry {
    Metainfo(Metadata),
    Table(Table),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Metadata {
    version: String,
    release_name: String,
    json_schema_version: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Table {
    family: String,
    name: String,
    handle: u64,
    #[serde(default)]
    flags: Vec<String>,
    #[serde(default)]
    comment: Option<String>,
}

fn absent(bytes: &[u8], name: &str) -> Result<(), ExitError> {
    let tables: Tables = serde_json::from_slice(bytes).map_err(|_| Reason::NftUnavailable)?;
    if tables.nftables.len() > 16384 {
        return Err(Reason::NftUnavailable.into());
    }
    let mut metadata = 0;
    for entry in tables.nftables {
        match entry {
            Entry::Metainfo(info) => {
                if info.version.is_empty()
                    || info.version.len() > 128
                    || info.release_name.len() > 128
                    || info.json_schema_version != 1
                {
                    return Err(Reason::NftUnavailable.into());
                }
                metadata += 1;
            }
            Entry::Table(table) => {
                if table.family.len() > 16
                    || table.name.len() > 256
                    || table.handle == 0
                    || table.flags.len() > 8
                    || table.comment.is_some_and(|c| c.len() > 128)
                {
                    return Err(Reason::NftUnavailable.into());
                }
                if table.family == "inet" && table.name == name {
                    return Err(Reason::OwnershipConflict.into());
                }
            }
        }
    }
    if metadata != 1 {
        return Err(Reason::NftUnavailable.into());
    }
    Ok(())
}

pub(super) async fn verify_absent(commands: &mut Commands, name: &str) -> Result<(), ExitError> {
    let output = commands
        .run(
            Tool::Nft,
            vec!["-j".into(), "list".into(), "tables".into()],
            Vec::new(),
        )
        .await
        .map_err(|_| Reason::NftUnavailable)?;
    if !output.success {
        return Err(Reason::NftUnavailable.into());
    }
    absent(&output.stdout, name)
}

#[cfg(test)]
mod tests;
