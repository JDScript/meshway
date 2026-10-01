//! Private local carrier material. Deliberately no Debug on any profile holder.

use std::{collections::BTreeMap, io::Read, path::Path, sync::Arc};

use meshway_protocol::{ConnectionTransport, NodeConnections};
use meshway_transport_vless::{ClientConfig, SecretUuid, ServerConfig, Target};
use rustls::{
    ClientConfig as TlsClient, RootCertStore, ServerConfig as TlsServer, pki_types::ServerName,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, Zeroizing};

use super::{Failure, Reason};

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

#[derive(Clone)]
pub(super) struct Server {
    pub tls: Arc<TlsServer>,
    pub vless: ServerConfig,
}

#[derive(Clone)]
pub(super) struct Client {
    pub tls: Arc<TlsClient>,
    pub name: ServerName<'static>,
    pub vless: ClientConfig,
}

#[derive(Default)]
pub(super) struct Profiles {
    pub servers: BTreeMap<String, Server>,
    pub clients: BTreeMap<String, Client>,
    pub fingerprint: [u8; 32],
}

fn unavailable<T>(_: T) -> Failure {
    Failure(Reason::ProfileUnavailable)
}

fn private_bytes(path: &Path, limit: usize) -> Result<Zeroizing<Vec<u8>>, Failure> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        // Profiles/key PEMs are read-only inputs, including Docker read-only
        // mounts. Check the opened descriptor, not a pre-open pathname stat.
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
    Err(Failure(Reason::ProfileUnavailable))
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
    Err(Failure(Reason::ProfileUnavailable))
}

fn certificates(bytes: &[u8]) -> Result<Vec<rustls::pki_types::CertificateDer<'static>>, Failure> {
    let certs = rustls_pemfile::certs(&mut std::io::Cursor::new(bytes))
        .collect::<Result<Vec<_>, _>>()
        .map_err(unavailable)?;
    if certs.is_empty() || certs.len() > 16 {
        return Err(Failure(Reason::ProfileUnavailable));
    }
    Ok(certs)
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
    let mut result = Profiles::default();
    let mut hash = Sha256::new();
    for (id, server) in needed {
        let profile = indexed
            .get(&id)
            .ok_or(Failure(Reason::ProfileUnavailable))?;
        let encoded = Zeroizing::new(serde_json::to_vec(profile).map_err(unavailable)?);
        hash_part(&mut hash, &encoded);
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let marker = Target::ip(std::net::Ipv4Addr::LOCALHOST.into(), 8421).map_err(unavailable)?;
        match profile {
            Profile::Server {
                uuid,
                certificate,
                private_key,
                ..
            } if server => {
                let uuid: SecretUuid = uuid.parse().map_err(unavailable)?;
                let cert = material(&parent.join(certificate), false)?;
                let key = material(&parent.join(private_key), true)?;
                hash_part(&mut hash, &cert);
                hash_part(&mut hash, &key);
                let mut cursor = std::io::Cursor::new(key.as_slice());
                let private_key = rustls_pemfile::private_key(&mut cursor)
                    .map_err(unavailable)?
                    .ok_or(Failure(Reason::ProfileUnavailable))?;
                if rustls_pemfile::private_key(&mut cursor)
                    .map_err(unavailable)?
                    .is_some()
                {
                    return Err(Failure(Reason::ProfileUnavailable));
                }
                let tls = TlsServer::builder_with_provider(provider)
                    .with_safe_default_protocol_versions()
                    .map_err(unavailable)?
                    .with_no_client_auth()
                    .with_single_cert(certificates(&cert)?, private_key)
                    .map_err(unavailable)?;
                result.servers.insert(
                    id,
                    Server {
                        tls: Arc::new(tls),
                        vless: ServerConfig {
                            uuid,
                            expected_target: marker,
                        },
                    },
                );
            }
            Profile::Client {
                uuid,
                ca_certificate,
                server_name,
                ..
            } if !server => {
                let uuid = uuid.parse().map_err(unavailable)?;
                let ca = material(&parent.join(ca_certificate), false)?;
                hash_part(&mut hash, &ca);
                let mut roots = RootCertStore::empty();
                for cert in certificates(&ca)? {
                    roots.add(cert).map_err(unavailable)?;
                }
                let name = ServerName::try_from(server_name.clone()).map_err(unavailable)?;
                let tls = TlsClient::builder_with_provider(provider)
                    .with_safe_default_protocol_versions()
                    .map_err(unavailable)?
                    .with_root_certificates(roots)
                    .with_no_client_auth();
                result.clients.insert(
                    id,
                    Client {
                        tls: Arc::new(tls),
                        name,
                        vless: ClientConfig {
                            uuid,
                            target: marker,
                        },
                    },
                );
            }
            _ => return Err(Failure(Reason::ProfileUnavailable)),
        }
    }
    result.fingerprint = hash.finalize().into();
    Ok(result)
}
