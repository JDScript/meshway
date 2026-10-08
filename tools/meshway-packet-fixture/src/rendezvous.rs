//! Public fixture identities and atomic rendezvous publication shared by actors.
use crate::Error;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(
    Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, clap::ValueEnum, Serialize, Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    A,
    R,
    B,
}

impl Role {
    pub const ALL: [Self; 3] = [Self::A, Self::R, Self::B];

    pub fn name(self) -> &'static str {
        match self {
            Self::A => "a",
            Self::R => "r",
            Self::B => "b",
        }
    }
}

/// Publish a complete public record only after its temporary file is synced.
pub async fn write<T: Serialize>(directory: &Path, name: &str, value: &T) -> Result<(), Error> {
    use tokio::io::AsyncWriteExt;
    let destination = directory.join(format!("{name}.json"));
    let temporary = directory.join(format!(".{name}.{}.tmp", std::process::id()));
    let bytes = serde_json::to_vec(value)?;
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .await?;
    file.write_all(&bytes).await?;
    file.sync_all().await?;
    drop(file);
    tokio::fs::rename(temporary, destination).await?;
    Ok(())
}
