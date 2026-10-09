//! The one strict, bounded decoder of `nft -j list tables`. Owners interpret
//! the returned identities; none of them is ownership by itself.
use super::command::{Commands, Tool};
use serde::Deserialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::linux::kernel) enum TablesError {
    /// The tool did not run or reported failure.
    Unavailable,
    /// Unknown JSON, a missing or duplicated metainfo, or out-of-bound values.
    Malformed,
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

/// Every `(family, name)` in listing order, duplicates included.
pub(in crate::linux::kernel) fn decode(bytes: &[u8]) -> Result<Vec<(String, String)>, TablesError> {
    let tables: Tables = serde_json::from_slice(bytes).map_err(|_| TablesError::Malformed)?;
    if tables.nftables.len() > 16384 {
        return Err(TablesError::Malformed);
    }
    let mut metadata = 0;
    let mut found = Vec::new();
    for entry in tables.nftables {
        match entry {
            Entry::Metainfo(info) => {
                if info.version.is_empty()
                    || info.version.len() > 128
                    || info.release_name.len() > 128
                    || info.json_schema_version != 1
                {
                    return Err(TablesError::Malformed);
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
                    return Err(TablesError::Malformed);
                }
                found.push((table.family, table.name));
            }
        }
    }
    if metadata != 1 {
        return Err(TablesError::Malformed);
    }
    Ok(found)
}

pub(in crate::linux::kernel) async fn tables(
    commands: &mut Commands,
) -> Result<Vec<(String, String)>, TablesError> {
    let output = commands
        .run(
            Tool::Nft,
            vec!["-j".into(), "list".into(), "tables".into()],
            Vec::new(),
        )
        .await
        .map_err(|_| TablesError::Unavailable)?;
    if !output.success {
        return Err(TablesError::Unavailable);
    }
    decode(&output.stdout)
}
