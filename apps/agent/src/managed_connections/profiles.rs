//! App-owned legacy profile file input. Carrier construction consumes memory only.
use super::{Failure, Reason};
use meshway_agent_connections::profiles::{LocalMaterial, Profiles, compile_local, with_managed};
use meshway_protocol::{ConnectionTransport, NodeConnections};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, io::Read, path::Path};
use zeroize::{Zeroize, Zeroizing};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    version: u32,
    profiles: Vec<Profile>,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "role", rename_all = "snake_case", deny_unknown_fields)]
enum Profile {
    Server {
        id: String,
        uuid: String,
        certificate: String,
        private_key: String,
    },
    Client {
        id: String,
        uuid: String,
        ca_certificate: String,
        server_name: String,
    },
}
impl Profile {
    fn id(&self) -> &str {
        match self {
            Self::Server { id, .. } | Self::Client { id, .. } => id,
        }
    }
}
impl Drop for Profile {
    fn drop(&mut self) {
        match self {
            Self::Server { uuid, .. } | Self::Client { uuid, .. } => uuid.zeroize(),
        }
    }
}
fn unavailable<T>(_: T) -> Failure {
    Failure(Reason::ProfileUnavailable)
}

fn private_bytes(path: &Path, limit: usize) -> Result<Zeroizing<Vec<u8>>, Failure> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(
                (rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK).bits() as i32,
            )
            .open(path)
            .map_err(unavailable)?;
        let metadata = file.metadata().map_err(unavailable)?;
        if !metadata.is_file()
            || metadata.uid() != rustix::process::geteuid().as_raw()
            || metadata.nlink() != 1
            || metadata.mode() & 0o777 != 0o600
        {
            return Err(Failure(Reason::ProfileUnavailable));
        }
        let mut bytes = Zeroizing::new(Vec::new());
        file.take(limit as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(unavailable)?;
        if bytes.len() > limit {
            return Err(Failure(Reason::ProfileUnavailable));
        }
        Ok(bytes)
    }
    #[cfg(not(unix))]
    {
        let _ = (path, limit);
        Err(Failure(Reason::ProfileUnavailable))
    }
}
fn material(path: &Path, private: bool) -> Result<Zeroizing<Vec<u8>>, Failure> {
    const LIMIT: usize = 256 * 1024;
    if private {
        return private_bytes(path, LIMIT);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(
                (rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK).bits() as i32,
            )
            .open(path)
            .map_err(unavailable)?;
        let metadata = file.metadata().map_err(unavailable)?;
        if !metadata.is_file()
            || metadata.uid() != rustix::process::geteuid().as_raw()
            || metadata.mode() & 0o022 != 0
            || metadata.nlink() != 1
        {
            return Err(Failure(Reason::ProfileUnavailable));
        }
        let mut bytes = Zeroizing::new(Vec::new());
        file.take(LIMIT as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(unavailable)?;
        if bytes.len() > LIMIT {
            return Err(Failure(Reason::ProfileUnavailable));
        }
        Ok(bytes)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Err(Failure(Reason::ProfileUnavailable))
    }
}
fn hash_part(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_be_bytes());
    hash.update(bytes);
}

pub(super) fn load(path: Option<&Path>, intent: &NodeConnections) -> Result<Profiles, Failure> {
    let mut needed = BTreeMap::new();
    for listener in &intent.listeners {
        if listener.transport == ConnectionTransport::Vless {
            let id = listener
                .credential_ref
                .as_ref()
                .ok_or(Failure(Reason::ProfileUnavailable))?;
            needed.insert(id.clone(), true);
        }
    }
    for candidate in &intent.outgoing {
        if candidate.transport == ConnectionTransport::Vless {
            let id = candidate
                .credential_ref
                .as_ref()
                .ok_or(Failure(Reason::ProfileUnavailable))?;
            if needed.insert(id.clone(), false) == Some(true) {
                return Err(Failure(Reason::ProfileUnavailable));
            }
        }
    }
    let Some(path) = path else {
        return if needed.is_empty() {
            Ok(Profiles::default())
        } else {
            Err(Failure(Reason::ProfileUnavailable))
        };
    };
    let bytes = private_bytes(path, 128 * 1024)?;
    let file: File = serde_json::from_slice(&bytes).map_err(unavailable)?;
    if file.version != 1 || file.profiles.len() > 128 {
        return Err(Failure(Reason::ProfileUnavailable));
    }
    let parent = path.parent().unwrap_or(Path::new("."));
    let mut indexed = BTreeMap::new();
    for profile in file.profiles {
        let id = profile.id();
        if id.is_empty()
            || id.len() > 64
            || !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
            || indexed.contains_key(id)
        {
            return Err(Failure(Reason::ProfileUnavailable));
        }
        indexed.insert(id.to_owned(), profile);
    }
    let mut hash = Sha256::new();
    let mut materials = Vec::new();
    for (id, server) in needed {
        let profile = indexed
            .get(&id)
            .ok_or(Failure(Reason::ProfileUnavailable))?;
        let encoded = Zeroizing::new(serde_json::to_vec(profile).map_err(unavailable)?);
        hash_part(&mut hash, &encoded);
        materials.push(match profile {
            Profile::Server {
                uuid,
                certificate,
                private_key,
                ..
            } if server => {
                let certificate = material(&parent.join(certificate), false)?;
                let private_key = material(&parent.join(private_key), true)?;
                hash_part(&mut hash, &certificate);
                hash_part(&mut hash, &private_key);
                LocalMaterial::Server {
                    id,
                    uuid: Zeroizing::new(uuid.clone()),
                    certificate,
                    private_key,
                }
            }
            Profile::Client {
                uuid,
                ca_certificate,
                server_name,
                ..
            } if !server => {
                let ca_certificate = material(&parent.join(ca_certificate), false)?;
                hash_part(&mut hash, &ca_certificate);
                LocalMaterial::Client {
                    id,
                    uuid: Zeroizing::new(uuid.clone()),
                    ca_certificate,
                    server_name: server_name.clone(),
                }
            }
            _ => return Err(Failure(Reason::ProfileUnavailable)),
        });
    }
    let mut profiles = compile_local(&materials)?;
    profiles.fingerprint = hash.finalize().into();
    Ok(profiles)
}

pub(super) fn load_combined(
    path: Option<&Path>,
    intent: &NodeConnections,
    materials: &[meshway_protocol::ManagedVlessMaterial],
) -> Result<Profiles, Failure> {
    let mut legacy = intent.clone();
    legacy.listeners.retain(|l| {
        !l.credential_ref
            .as_deref()
            .is_some_and(|r| r.starts_with("managed."))
    });
    legacy.outgoing.retain(|c| {
        !c.credential_ref
            .as_deref()
            .is_some_and(|r| r.starts_with("managed."))
    });
    with_managed(load(path, &legacy)?, intent, materials)
}
