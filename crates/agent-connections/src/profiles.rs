//! In-memory carrier material and TLS/VLESS construction. No local file input.
//! Deliberately no Debug on any secret-bearing holder.
use super::{Failure, Reason};
use meshway_protocol::{ConnectionTransport, NodeConnections};
use meshway_transport_vless::{ClientConfig, SecretUuid, ServerConfig, Target};
use rustls::{
    ClientConfig as TlsClient, RootCertStore, ServerConfig as TlsServer, pki_types::ServerName,
};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use zeroize::Zeroizing;

pub enum LocalMaterial {
    Server {
        id: String,
        uuid: Zeroizing<String>,
        certificate: Zeroizing<Vec<u8>>,
        private_key: Zeroizing<Vec<u8>>,
    },
    Client {
        id: String,
        uuid: Zeroizing<String>,
        ca_certificate: Zeroizing<Vec<u8>>,
        server_name: String,
    },
}

#[derive(Clone)]
pub struct Server {
    pub tls: Arc<TlsServer>,
    pub vless: ServerConfig,
    pub websocket: Option<meshway_transport_vless::websocket::Parameters>,
}
#[derive(Clone)]
pub struct Client {
    pub tls: Arc<TlsClient>,
    pub name: ServerName<'static>,
    pub vless: ClientConfig,
    pub websocket: Option<meshway_transport_vless::websocket::Parameters>,
}
#[derive(Default)]
pub struct Profiles {
    pub servers: BTreeMap<String, Server>,
    pub clients: BTreeMap<String, Client>,
    pub fingerprint: [u8; 32],
}
fn unavailable<T>(_: T) -> Failure {
    Failure(Reason::ProfileUnavailable)
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
fn private_key(bytes: &[u8]) -> Result<rustls::pki_types::PrivateKeyDer<'static>, Failure> {
    let mut cursor = std::io::Cursor::new(bytes);
    let key = rustls_pemfile::private_key(&mut cursor)
        .map_err(unavailable)?
        .ok_or(Failure(Reason::ProfileUnavailable))?;
    if rustls_pemfile::private_key(&mut cursor)
        .map_err(unavailable)?
        .is_some()
    {
        return Err(Failure(Reason::ProfileUnavailable));
    }
    Ok(key)
}

/// Compile typed memory inputs. The App owns selection, file checks and its
/// source fingerprint; no local paths or legacy file schema cross this boundary.
pub fn compile_local(materials: &[LocalMaterial]) -> Result<Profiles, Failure> {
    let mut result = Profiles::default();
    let mut seen = BTreeSet::new();
    for material in materials {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let marker = Target::ip(std::net::Ipv4Addr::LOCALHOST.into(), 8421).map_err(unavailable)?;
        let id = match material {
            LocalMaterial::Server { id, .. } | LocalMaterial::Client { id, .. } => id,
        };
        if id.is_empty() || !seen.insert(id) {
            return Err(Failure(Reason::ProfileUnavailable));
        }
        match material {
            LocalMaterial::Server {
                id,
                uuid,
                certificate,
                private_key: key,
            } => {
                let uuid: SecretUuid = uuid.as_str().parse().map_err(unavailable)?;
                let tls = TlsServer::builder_with_provider(provider)
                    .with_safe_default_protocol_versions()
                    .map_err(unavailable)?
                    .with_no_client_auth()
                    .with_single_cert(certificates(certificate)?, private_key(key)?)
                    .map_err(unavailable)?;
                result.servers.insert(
                    id.clone(),
                    Server {
                        tls: Arc::new(tls),
                        vless: ServerConfig {
                            uuid,
                            expected_target: marker,
                        },
                        websocket: None,
                    },
                );
            }
            LocalMaterial::Client {
                id,
                uuid,
                ca_certificate,
                server_name,
            } => {
                let uuid = uuid.as_str().parse().map_err(unavailable)?;
                let mut roots = RootCertStore::empty();
                for cert in certificates(ca_certificate)? {
                    roots.add(cert).map_err(unavailable)?;
                }
                let name = ServerName::try_from(server_name.clone()).map_err(unavailable)?;
                let tls = TlsClient::builder_with_provider(provider)
                    .with_safe_default_protocol_versions()
                    .map_err(unavailable)?
                    .with_root_certificates(roots)
                    .with_no_client_auth();
                result.clients.insert(
                    id.clone(),
                    Client {
                        tls: Arc::new(tls),
                        name,
                        vless: ClientConfig {
                            uuid,
                            target: marker,
                        },
                        websocket: None,
                    },
                );
            }
        }
    }
    Ok(result)
}

pub fn with_managed(
    mut result: Profiles,
    intent: &NodeConnections,
    materials: &[meshway_protocol::ManagedVlessMaterial],
) -> Result<Profiles, Failure> {
    use meshway_protocol::VlessStreamTransport;
    let mut hash = Sha256::new();
    hash_part(&mut hash, &result.fingerprint);
    let mut seen = BTreeSet::new();
    for material in materials {
        let id = &material.credential_ref;
        let expected = format!("managed.{}", material.listener_id);
        let owner = intent.listeners.iter().any(|l| {
            l.id == material.listener_id
                && l.transport == ConnectionTransport::Vless
                && l.credential_ref.as_ref() == Some(id)
        });
        let dialer = intent.outgoing.iter().any(|c| {
            c.listener_id == material.listener_id
                && c.transport == ConnectionTransport::Vless
                && c.credential_ref.as_ref() == Some(id)
        });
        if id != &expected
            || !seen.insert(id.clone())
            || material.version == 0
            || (!owner && !dialer)
            || owner != material.private_key_pem.is_some()
        {
            return Err(Failure(Reason::ProfileUnavailable));
        }
        let certs = certificates(material.certificate_pem.as_bytes())?;
        let fingerprint: String = Sha256::digest(certs[0].as_ref())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        if fingerprint != material.certificate_sha256 {
            return Err(Failure(Reason::ProfileUnavailable));
        }
        let websocket = match material.settings.stream {
            VlessStreamTransport::Tcp
                if material.settings.websocket_path.is_none()
                    && material.settings.websocket_host.is_none() =>
            {
                None
            }
            VlessStreamTransport::Websocket => {
                let value = meshway_transport_vless::websocket::Parameters {
                    path: material
                        .settings
                        .websocket_path
                        .clone()
                        .ok_or(Failure(Reason::ProfileUnavailable))?,
                    host: material
                        .settings
                        .websocket_host
                        .clone()
                        .ok_or(Failure(Reason::ProfileUnavailable))?,
                };
                value.validate().map_err(unavailable)?;
                Some(value)
            }
            _ => return Err(Failure(Reason::ProfileUnavailable)),
        };
        let encoded = Zeroizing::new(serde_json::to_vec(material).map_err(unavailable)?);
        hash_part(&mut hash, &encoded);
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let mut roots = RootCertStore::empty();
        roots
            .add(
                certs
                    .last()
                    .ok_or(Failure(Reason::ProfileUnavailable))?
                    .clone(),
            )
            .map_err(unavailable)?;
        let verifier = rustls::client::WebPkiServerVerifier::builder_with_provider(
            Arc::new(roots),
            provider.clone(),
        )
        .build()
        .map_err(unavailable)?;
        let name =
            ServerName::try_from(material.settings.server_name.clone()).map_err(unavailable)?;
        use rustls::client::danger::ServerCertVerifier;
        verifier
            .verify_server_cert(
                &certs[0],
                &certs[1..],
                &name,
                &[],
                rustls::pki_types::UnixTime::now(),
            )
            .map_err(unavailable)?;
        let uuid: SecretUuid = material.uuid.parse().map_err(unavailable)?;
        let marker = Target::ip(std::net::Ipv4Addr::LOCALHOST.into(), 8421).map_err(unavailable)?;
        if owner {
            let key = material
                .private_key_pem
                .as_ref()
                .ok_or(Failure(Reason::ProfileUnavailable))?;
            let tls = TlsServer::builder_with_provider(provider.clone())
                .with_safe_default_protocol_versions()
                .map_err(unavailable)?
                .with_no_client_auth()
                .with_single_cert(certs.clone(), private_key(key.as_bytes())?)
                .map_err(unavailable)?;
            result.servers.insert(
                id.clone(),
                Server {
                    tls: Arc::new(tls),
                    vless: ServerConfig {
                        uuid: uuid.clone(),
                        expected_target: marker.clone(),
                    },
                    websocket: websocket.clone(),
                },
            );
        }
        if dialer {
            let mut roots = RootCertStore::empty();
            for cert in certs {
                roots.add(cert).map_err(unavailable)?;
            }
            let tls = TlsClient::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .map_err(unavailable)?
                .with_root_certificates(roots)
                .with_no_client_auth();
            let name =
                ServerName::try_from(material.settings.server_name.clone()).map_err(unavailable)?;
            result.clients.insert(
                id.clone(),
                Client {
                    tls: Arc::new(tls),
                    name,
                    vless: ClientConfig {
                        uuid,
                        target: marker,
                    },
                    websocket,
                },
            );
        }
    }
    if intent
        .listeners
        .iter()
        .filter_map(|l| l.credential_ref.as_ref())
        .chain(
            intent
                .outgoing
                .iter()
                .filter_map(|c| c.credential_ref.as_ref()),
        )
        .any(|r| r.starts_with("managed.") && !seen.contains(r))
    {
        return Err(Failure(Reason::ProfileUnavailable));
    }
    result.fingerprint = hash.finalize().into();
    Ok(result)
}
