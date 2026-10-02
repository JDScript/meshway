//! Network-scoped public settings and encrypted recoverable carrier materials.
use super::{NetworkId, Store, configuration, connections};
use crate::Error;
use chacha20poly1305::{
    ChaCha20Poly1305, KeyInit,
    aead::{Aead, Payload},
};
use meshway_protocol::*;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use sha2::{Digest, Sha256};
use std::path::Path;
use zeroize::Zeroizing;
#[cfg(test)]
mod tests;

pub(super) fn upgrade(connection: &mut Connection) -> Result<(), Error> {
    let version: u32 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version == 10 {
        return Ok(());
    }
    if version != 9 {
        return Err(Error::Configuration("managed transports require schema 9"));
    }
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute_batch(include_str!("schema_v10.sql"))?;
    if tx.prepare("PRAGMA foreign_key_check")?.exists([])? {
        return Err(Error::Configuration(
            "managed transport migration foreign key failure",
        ));
    }
    tx.commit()?;
    Ok(())
}

pub(super) fn key(path: &Path, connection: &Connection) -> Result<Zeroizing<Vec<u8>>, Error> {
    let file = path.join("transport-material.key");
    if !file.try_exists()? {
        if connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM managed_listeners WHERE material_ciphertext IS NOT NULL)",
            [],
            |r| r.get::<_, bool>(0),
        )? {
            return Err(Error::Configuration(
                "missing existing transport material key; refusing replacement",
            ));
        }
        local::atomic_write(&file, local::credential()?.as_bytes(), false)?;
    }
    let encoded = Zeroizing::new(local::read_private(&file)?);
    if !local::valid_credential(&encoded) {
        return Err(Error::Configuration("invalid transport material key"));
    }
    local::ensure_durable(&file)?;
    Ok(Zeroizing::new(
        (0..32)
            .map(|i| u8::from_str_radix(&encoded[i * 2..i * 2 + 2], 16).expect("validated hex"))
            .collect(),
    ))
}

fn aad(network: &NetworkId, id: &str, node: &str, version: u32) -> Vec<u8> {
    format!(
        "meshway-material-v1:{}:{id}:{node}:{version}",
        network.as_str()
    )
    .into_bytes()
}

fn reference(id: &str) -> String {
    format!("managed.{id}")
}

pub(super) fn listeners(
    connection: &Connection,
    network: &NetworkId,
) -> Result<Vec<ManagedListenerSummary>, Error> {
    let mut result = Vec::new();
    let mut statement = connection.prepare("SELECT settings,material_version,certificate_sha256 FROM managed_listeners WHERE network_id=?1 ORDER BY listener_id")?;
    for row in statement.query_map([network.as_str()], |r| {
        Ok((r.get::<_, String>(0)?, r.get(1)?, r.get(2)?))
    })? {
        let (json, material_version, certificate_sha256) = row?;
        let settings: ManagedListenerSettings = serde_json::from_str(&json)
            .map_err(|_| Error::Configuration("invalid managed listener settings"))?;
        let capabilities: Option<String> = connection
            .query_row(
                "SELECT capabilities FROM connection_identities WHERE network_id=?1 AND node_id=?2",
                params![network.as_str(), settings.node_id],
                |r| r.get(0),
            )
            .optional()?;
        let capabilities: Vec<ConnectionCapability> = capabilities
            .map(|json| serde_json::from_str(&json))
            .transpose()
            .map_err(|_| Error::Configuration("invalid capabilities"))?
            .unwrap_or_default();
        let active: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM nodes WHERE network_id=?1 AND id=?2 AND revoked_at IS NULL)",params![network.as_str(),settings.node_id],|r|r.get(0))?;
        let streams:Option<String>=connection.query_row("SELECT vless_streams FROM managed_transport_capabilities WHERE network_id=?1 AND node_id=?2",params![network.as_str(),settings.node_id],|r|r.get(0)).optional()?;
        let streams: Vec<VlessStreamTransport> = streams
            .map(|s| serde_json::from_str(&s))
            .transpose()
            .map_err(|_| Error::Configuration("invalid managed capability report"))?
            .unwrap_or_default();
        let available = active
            && capabilities
                .iter()
                .any(|c| c.transport == settings.transport && c.can_accept)
            && settings
                .vless
                .as_ref()
                .is_none_or(|v| streams.contains(&v.stream));
        result.push(ManagedListenerSummary {
            settings,
            material_version,
            certificate_sha256,
            compiled_capability_available: available,
        });
    }
    Ok(result)
}

/// Public v1 projection uses the existing reference slot; no secrets are added.
pub(super) fn append(
    connection: &Connection,
    network: &NetworkId,
    plan: &mut ConnectionPlan,
) -> Result<(), Error> {
    for summary in listeners(connection, network)? {
        let item = summary.settings;
        if !item.enabled {
            continue;
        }
        if plan.listeners.iter().any(|l| l.id == item.id) {
            return Err(Error::Configuration(
                "managed and legacy listener IDs collide",
            ));
        }
        plan.listeners.push(ConnectionListener {
            id: item.id.clone(),
            node_id: item.node_id,
            transport: item.transport,
            bind: std::net::SocketAddr::new(
                item.bind_address
                    .parse()
                    .map_err(|_| Error::Configuration("invalid managed bind"))?,
                item.bind_port,
            )
            .to_string(),
            advertised_endpoints: item.advertised_endpoints,
            credential_ref: (item.transport == ConnectionTransport::Vless)
                .then(|| reference(&item.id)),
        });
    }
    for candidate in &mut plan.candidates {
        if let Some(listener) = plan.listeners.iter().find(|l| {
            l.id == candidate.listener_id
                && l.credential_ref.as_deref() == Some(reference(&l.id).as_str())
        }) {
            candidate.credential_ref = listener.credential_ref.clone();
        }
    }
    Ok(())
}

/// Managed stream support is executable compilation input, not a UI hint.
/// Retain intent but let the existing whole-plan fail-closed projection withdraw
/// every listener/candidate/material until owner AND all dialers support it.
pub(super) fn stream_diagnostics(
    connection: &Connection,
    network: &NetworkId,
    plan: &ConnectionPlan,
) -> Result<Vec<ConnectionDiagnostic>, Error> {
    let mut diagnostics = Vec::new();
    let mut statement = connection.prepare(
        "SELECT node_id,vless_streams FROM managed_transport_capabilities WHERE network_id=?1",
    )?;
    let mut streams = std::collections::BTreeMap::new();
    for row in statement.query_map([network.as_str()], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })? {
        let (id, json) = row?;
        let modes: Vec<VlessStreamTransport> = serde_json::from_str(&json)
            .map_err(|_| Error::Configuration("invalid managed stream capabilities"))?;
        streams.insert(id, modes);
    }
    for listener in listeners(connection, network)? {
        if !listener.settings.enabled {
            continue;
        }
        let Some(vless) = &listener.settings.vless else {
            continue;
        };
        if !listener.compiled_capability_available {
            diagnostics.push(ConnectionDiagnostic{item_id:Some(listener.settings.id.clone()),field:"listeners.vless.stream".into(),message:"Listener owner has not declared the required managed VLESS stream and accept capability".into()});
        }
        for candidate in plan
            .candidates
            .iter()
            .filter(|c| c.listener_id == listener.settings.id)
        {
            if streams
                .get(&candidate.dialer_node_id)
                .is_none_or(|modes| !modes.contains(&vless.stream))
            {
                diagnostics.push(ConnectionDiagnostic {
                    item_id: Some(candidate.id.clone()),
                    field: "candidates.vless.stream".into(),
                    message: "Dialer has not declared the required managed VLESS stream capability"
                        .into(),
                });
            }
        }
    }
    diagnostics.truncate(64);
    Ok(diagnostics)
}

fn validate(settings: &ManagedListenerSettings) -> Result<(), Error> {
    let id = &settings.id;
    if id.is_empty()
        || id.len() > 48
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        || id.starts_with("managed.")
    {
        return Err(Error::Unprocessable("invalid listener ID"));
    }
    let ip: std::net::IpAddr = settings
        .bind_address
        .parse()
        .map_err(|_| Error::Unprocessable("bind_address must be an IP address"))?;
    if !super::connections::validation::usable_ip(ip, true)
        || settings.bind_port == 0
        || settings.advertised_endpoints.len() > 16
    {
        return Err(Error::Unprocessable("invalid bind or endpoints"));
    }
    for endpoint in &settings.advertised_endpoints {
        super::connections::validation::endpoint(endpoint).map_err(Error::Unprocessable)?;
    }
    match (settings.transport, &settings.vless) {
        (ConnectionTransport::Vless, Some(vless)) => {
            let name = &vless.server_name;
            if name.is_empty()
                || name.len() > 253
                || name
                    .bytes()
                    .any(|b| !b.is_ascii_alphanumeric() && b != b'.' && b != b'-')
                || name
                    .split('.')
                    .any(|l| l.is_empty() || l.len() > 63 || l.starts_with('-') || l.ends_with('-'))
            {
                return Err(Error::Unprocessable("invalid TLS server_name"));
            }
            match vless.stream {
                VlessStreamTransport::Tcp
                    if vless.websocket_path.is_none() && vless.websocket_host.is_none() => {}
                VlessStreamTransport::Websocket => {
                    let path = vless
                        .websocket_path
                        .as_deref()
                        .ok_or(Error::Unprocessable("WS requires path"))?;
                    let host = vless
                        .websocket_host
                        .as_deref()
                        .ok_or(Error::Unprocessable("WS requires Host"))?;
                    if !path.starts_with('/')
                        || path.len() > 1024
                        || path.contains(['\r', '\n', '#', '?', ' '])
                        || host.is_empty()
                        || host.len() > 253
                        || !host.is_ascii()
                        || host.contains(['/', '\\', '?', '#', '@'])
                        || host
                            .bytes()
                            .any(|b| b.is_ascii_control() || b.is_ascii_whitespace())
                    {
                        return Err(Error::Unprocessable("invalid WebSocket path or Host"));
                    }
                    let uri = format!("wss://{host}{path}")
                        .parse::<axum::http::Uri>()
                        .map_err(|_| Error::Unprocessable("invalid WebSocket upgrade URI"))?;
                    if uri.authority().map(|a| a.as_str()) != Some(host)
                        || uri.path_and_query().map(|p| p.as_str()) != Some(path)
                    {
                        return Err(Error::Unprocessable("invalid WebSocket upgrade URI"));
                    }
                }
                _ => {
                    return Err(Error::Unprocessable(
                        "TCP must not contain WebSocket parameters",
                    ));
                }
            }
        }
        (ConnectionTransport::NativeUdp | ConnectionTransport::NativeTcp, None) => {}
        _ => return Err(Error::Unprocessable("VLESS settings must match transport")),
    }
    Ok(())
}

fn normalize(settings: &ManagedListenerSettings) -> Result<ManagedListenerSettings, Error> {
    validate(settings)?;
    let mut result = settings.clone();
    result.bind_address = result
        .bind_address
        .parse::<std::net::IpAddr>()
        .map_err(|_| Error::Unprocessable("invalid bind address"))?
        .to_string();
    result.advertised_endpoints = result
        .advertised_endpoints
        .iter()
        .map(|e| super::connections::validation::endpoint(e).map_err(Error::Unprocessable))
        .collect::<Result<Vec<_>, _>>()?;
    result.advertised_endpoints.sort();
    if result.advertised_endpoints.windows(2).any(|e| e[0] == e[1]) {
        return Err(Error::Unprocessable(
            "duplicate normalized advertised endpoints",
        ));
    }
    if let Some(vless) = &mut result.vless {
        vless.server_name = vless.server_name.to_ascii_lowercase();
    }
    Ok(result)
}

fn generate(
    id: &str,
    version: u32,
    settings: &ManagedVlessSettings,
) -> Result<ManagedVlessMaterial, Error> {
    let rcgen::CertifiedKey { cert, signing_key } =
        rcgen::generate_simple_self_signed(vec![settings.server_name.clone()])
            .map_err(|_| Error::Configuration("cannot generate transport certificate"))?;
    let random = local::credential()?;
    let variant = 8 | (u8::from_str_radix(&random[16..17], 16).expect("random hex") & 3);
    let uuid = format!(
        "{}-{}-4{}-{variant:x}{}-{}",
        &random[..8],
        &random[8..12],
        &random[13..16],
        &random[17..20],
        &random[20..32]
    );
    let certificate_sha256 = Sha256::digest(cert.der())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok(ManagedVlessMaterial {
        listener_id: id.into(),
        credential_ref: reference(id),
        version,
        settings: settings.clone(),
        uuid,
        certificate_pem: cert.pem(),
        certificate_sha256,
        private_key_pem: Some(signing_key.serialize_pem()),
    })
}

fn decrypt(
    connection: &Connection,
    network: &NetworkId,
    id: &str,
    key: &[u8],
) -> Result<Option<ManagedVlessMaterial>, Error> {
    type EncryptedRow = (String, u32, Option<Vec<u8>>, Option<Vec<u8>>);
    let row:Option<EncryptedRow>=connection.query_row("SELECT node_id,material_version,material_nonce,material_ciphertext FROM managed_listeners WHERE network_id=?1 AND listener_id=?2",params![network.as_str(),id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
    let Some((node, version, Some(nonce), Some(ciphertext))) = row else {
        return Ok(None);
    };
    if nonce.len() != 12 {
        return Err(Error::Configuration("invalid stored material nonce"));
    }
    let cipher = ChaCha20Poly1305::new_from_slice(key)
        .map_err(|_| Error::Configuration("invalid material key"))?;
    let bytes = Zeroizing::new(
        cipher
            .decrypt(
                nonce.as_slice().into(),
                Payload {
                    msg: &ciphertext,
                    aad: &aad(network, id, &node, version),
                },
            )
            .map_err(|_| Error::Configuration("transport material authentication failed"))?,
    );
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| Error::Configuration("invalid stored transport material"))
}

pub(super) fn materials(
    connection: &Connection,
    network: &NetworkId,
    intent: &NodeConnections,
    key: &[u8],
) -> Result<Vec<ManagedVlessMaterial>, Error> {
    let mut result = Vec::new();
    for summary in listeners(connection, network)? {
        let id = &summary.settings.id;
        let (owner, dialer) = material_roles(&summary.settings, intent);
        if !owner && !dialer {
            continue;
        }
        let mut material = decrypt(connection, network, id, key)?
            .ok_or(Error::Configuration("missing managed material"))?;
        if !owner && let Some(mut key) = material.private_key_pem.take() {
            use zeroize::Zeroize;
            key.zeroize();
        }
        result.push(material);
    }
    Ok(result)
}

fn material_roles(settings: &ManagedListenerSettings, intent: &NodeConnections) -> (bool, bool) {
    if !settings.enabled || settings.transport != ConnectionTransport::Vless {
        return (false, false);
    }
    let id = &settings.id;
    let credential = reference(id);
    (
        intent.listeners.iter().any(|l| {
            l.id == *id
                && l.transport == ConnectionTransport::Vless
                && l.credential_ref.as_deref() == Some(credential.as_str())
        }),
        intent.outgoing.iter().any(|c| {
            c.listener_id == *id
                && c.transport == ConnectionTransport::Vless
                && c.credential_ref.as_deref() == Some(credential.as_str())
        }),
    )
}

/// Count exactly the same enabled managed owner/outgoing projection as download.
/// Each listener contributes once even if several candidate endpoints refer to it.
pub(super) fn material_count(
    connection: &Connection,
    network: &NetworkId,
    intent: &NodeConnections,
) -> Result<usize, Error> {
    Ok(listeners(connection, network)?
        .iter()
        .filter(|s| {
            let (owner, dialer) = material_roles(&s.settings, intent);
            owner || dialer
        })
        .count())
}

/// Encrypted plaintext-size upper bound. Owner material includes its key;
/// dialer projections are smaller. This avoids decrypting/logging material in
/// the network's publication-budget check and reserves the wrapper metadata.
pub(super) fn material_budget(
    connection: &Connection,
    network: &NetworkId,
    intent: &NodeConnections,
) -> Result<usize, Error> {
    let ids: std::collections::BTreeSet<_> = intent
        .listeners
        .iter()
        .filter(|l| l.transport == ConnectionTransport::Vless)
        .map(|l| l.id.as_str())
        .chain(
            intent
                .outgoing
                .iter()
                .filter(|c| c.transport == ConnectionTransport::Vless)
                .map(|c| c.listener_id.as_str()),
        )
        .collect();
    let mut total = 512usize;
    for id in ids {
        let size:Option<i64>=connection.query_row("SELECT length(material_ciphertext) FROM managed_listeners WHERE network_id=?1 AND listener_id=?2 AND material_ciphertext IS NOT NULL",params![network.as_str(),id],|r|r.get(0)).optional()?;
        let size = usize::try_from(size.unwrap_or(0))
            .map_err(|_| Error::Capacity("managed material publication budget exhausted"))?;
        total = total.checked_add(size).ok_or(Error::Capacity(
            "managed material publication budget exhausted",
        ))?;
    }
    Ok(total)
}

impl Store {
    pub(crate) fn verify_legacy_material(
        &self,
        network: &NetworkId,
        settings: &ManagedListenerSettings,
        material: &ManagedVlessMaterial,
        profiles: &Path,
    ) -> Result<(), Error> {
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct LegacyFile {
            version: u32,
            profiles: Vec<LegacyProfile>,
        }
        #[derive(serde::Deserialize)]
        #[serde(tag = "role", rename_all = "snake_case", deny_unknown_fields)]
        enum LegacyProfile {
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
        let invalid = || {
            Error::Unprocessable(
                "legacy profile/material equivalence could not be proven; input preserved",
            )
        };
        let source = connections::source(&self.connection, network)?;
        let listener = source
            .listeners
            .iter()
            .find(|l| {
                l.id == settings.id
                    && l.node_id == settings.node_id
                    && l.transport == ConnectionTransport::Vless
            })
            .ok_or_else(invalid)?;
        let reference = listener.credential_ref.as_ref().ok_or_else(invalid)?;
        let bytes = Zeroizing::new(local::read_private_bounded(profiles, 128 * 1024)?);
        let file: LegacyFile = serde_json::from_str(&bytes).map_err(|_| invalid())?;
        if file.version != 1 || file.profiles.len() > 128 {
            return Err(invalid());
        }
        let mut found = false;
        for profile in file.profiles {
            match profile {
                LegacyProfile::Server {
                    id,
                    uuid,
                    certificate,
                    private_key,
                } if &id == reference => {
                    if found || !uuid.eq_ignore_ascii_case(&material.uuid) {
                        return Err(invalid());
                    }
                    found = true;
                    let parent = profiles.parent().unwrap_or(Path::new("."));
                    let cert = Zeroizing::new(local::read_private_bounded(
                        &parent.join(certificate),
                        32768,
                    )?);
                    let key = Zeroizing::new(local::read_private_bounded(
                        &parent.join(private_key),
                        16384,
                    )?);
                    if !material.certificate_pem.starts_with(cert.as_str())
                        || material.private_key_pem.as_deref() != Some(key.as_str())
                    {
                        return Err(invalid());
                    }
                }
                LegacyProfile::Client {
                    id,
                    uuid,
                    ca_certificate,
                    server_name,
                } => {
                    let _ = (id, uuid, ca_certificate, server_name);
                }
                _ => {}
            }
        }
        if !found {
            return Err(invalid());
        }
        Ok(())
    }
    pub fn publish_managed_capabilities(
        &mut self,
        token: &str,
        request: &PublishManagedTransportCapabilitiesRequest,
    ) -> Result<PublishManagedTransportCapabilitiesResponse, Error> {
        if request.vless_streams.len() > 2
            || (request.vless_streams.len() == 2
                && request.vless_streams[0] == request.vless_streams[1])
        {
            return Err(Error::Invalid(
                "duplicate or excessive managed stream capabilities",
            ));
        }
        let network = self.node_network(token)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let id = configuration::node_id(&tx, &network, token)?;
        let mut streams = request.vless_streams.clone();
        streams.sort_by_key(|s| match s {
            VlessStreamTransport::Tcp => 0,
            VlessStreamTransport::Websocket => 1,
        });
        let encoded = serde_json::to_string(&streams)
            .map_err(|_| Error::Configuration("cannot encode managed capabilities"))?;
        let old:Option<String>=tx.query_row("SELECT vless_streams FROM managed_transport_capabilities WHERE network_id=?1 AND node_id=?2",params![network.as_str(),id],|r|r.get(0)).optional()?;
        if old.as_ref() != Some(&encoded) {
            tx.execute("INSERT INTO managed_transport_capabilities(network_id,node_id,vless_streams) VALUES(?1,?2,?3) ON CONFLICT(network_id,node_id) DO UPDATE SET vless_streams=excluded.vless_streams",params![network.as_str(),id,encoded])?;
            // Unused carrier facts do not mutate legacy/public execution
            // snapshots. Once managed VLESS exists this becomes publication
            // input and must fence the network's old generation.
            if listeners(&tx, &network)?
                .iter()
                .any(|l| l.settings.enabled && l.settings.transport == ConnectionTransport::Vless)
            {
                configuration::bump(&tx, &network)?;
            }
            super::publication::finish_change(
                &tx,
                &network,
                super::publication::ChangeKind::Adjacency,
            )?;
        }
        let revision = configuration::revision(&tx, &network)?;
        tx.commit()?;
        Ok(PublishManagedTransportCapabilitiesResponse { revision })
    }
    pub fn create_managed_token(
        &mut self,
        network: &NetworkId,
        request: &CreateManagedEnrollmentTokenRequest,
    ) -> Result<CreateEnrollmentTokenResponse, Error> {
        let initial_listener = normalize(&request.initial_listener)?;
        if !request.initial_listener.node_id.is_empty() {
            return Err(Error::Invalid(
                "initial listener node_id must be empty; identity comes from token consumption",
            ));
        }
        if !(60..=86400).contains(&request.expires_in_seconds) {
            return Err(Error::Invalid("invalid enrollment TTL"));
        }
        let tx = super::account_authorization::transaction(
            &mut self.connection,
            TransactionBehavior::Immediate,
        )?;
        let token = local::credential()?;
        let token_id = local::credential()?;
        let expires_at =
            chrono::Utc::now() + chrono::Duration::seconds(request.expires_in_seconds.into());
        tx.execute("INSERT INTO enrollment_tokens(id,token_hash,expires_at,network_id) VALUES(?1,?2,?3,?4)",params![token_id,local::digest(&token),expires_at.timestamp_millis(),network.as_str()])?;
        tx.execute(
            "INSERT INTO managed_enrollment_intents(network_id,token_id,settings) VALUES(?1,?2,?3)",
            params![
                network.as_str(),
                token_id,
                serde_json::to_string(&initial_listener)
                    .map_err(|_| Error::Configuration("cannot encode initial listener"))?
            ],
        )?;
        tx.commit()?;
        Ok(CreateEnrollmentTokenResponse {
            token_id,
            token,
            expires_at,
        })
    }
    pub fn managed_listeners(
        &self,
        network: &NetworkId,
    ) -> Result<ManagedListenersSnapshot, Error> {
        Ok(ManagedListenersSnapshot {
            revision: configuration::revision(&self.connection, network)?,
            listeners: listeners(&self.connection, network)?,
        })
    }
    pub fn save_managed_listener(
        &mut self,
        network: &NetworkId,
        request: &SaveManagedListenerRequest,
    ) -> Result<ManagedListenersSnapshot, Error> {
        self.save_managed_listener_inner(network, request, None)
    }
    pub(crate) fn import_managed_listener(
        &mut self,
        network: &NetworkId,
        request: &SaveManagedListenerRequest,
        material: ManagedVlessMaterial,
    ) -> Result<ManagedListenersSnapshot, Error> {
        validate_import(&request.settings, &material)?;
        self.save_managed_listener_inner(network, request, Some(material))
    }
    fn save_managed_listener_inner(
        &mut self,
        network: &NetworkId,
        request: &SaveManagedListenerRequest,
        imported: Option<ManagedVlessMaterial>,
    ) -> Result<ManagedListenersSnapshot, Error> {
        let normalized = normalize(&request.settings)?;
        let item = &normalized;
        let tx = super::account_authorization::transaction(
            &mut self.connection,
            TransactionBehavior::Immediate,
        )?;
        if configuration::revision(&tx, network)? != request.expected_revision {
            return Err(Error::Conflict);
        }
        if !tx.query_row("SELECT EXISTS(SELECT 1 FROM nodes WHERE network_id=?1 AND id=?2 AND revoked_at IS NULL)",params![network.as_str(),item.node_id],|r|r.get::<_,bool>(0))? {return Err(Error::NotFound);}
        let existing = listeners(&tx, network)?
            .into_iter()
            .find(|l| l.settings.id == item.id);
        if existing.as_ref().is_some_and(|l| {
            l.settings.node_id != item.node_id || l.settings.transport != item.transport
        }) {
            return Err(Error::Conflict);
        }
        let mut legacy = connections::source(&tx, network)?;
        if let Some(old) = legacy.listeners.iter().find(|l| l.id == item.id) {
            let bind = std::net::SocketAddr::new(
                item.bind_address
                    .parse()
                    .map_err(|_| Error::Unprocessable("invalid bind"))?,
                item.bind_port,
            )
            .to_string();
            if imported.is_none()
                || old.node_id != item.node_id
                || old.transport != item.transport
                || old.bind != bind
                || old.advertised_endpoints != item.advertised_endpoints
                || item
                    .vless
                    .as_ref()
                    .is_none_or(|v| v.stream != VlessStreamTransport::Tcp)
            {
                return Err(Error::Conflict);
            }
            legacy.listeners.retain(|l| l.id != item.id);
            for candidate in &mut legacy.candidates {
                if candidate.listener_id == item.id {
                    candidate.credential_ref = Some(reference(&item.id));
                }
            }
            tx.execute(
                "UPDATE connection_source SET configuration=?1 WHERE network_id=?2",
                params![
                    serde_json::to_string(&legacy)
                        .map_err(|_| Error::Configuration("cannot encode imported source"))?,
                    network.as_str()
                ],
            )?;
        }
        if imported.is_some() && existing.is_some() {
            return Err(Error::Conflict);
        }
        if existing.as_ref().is_none_or(|l| l.settings != *item) {
            if existing.is_none() && listeners(&tx, network)?.len() >= 128 {
                return Err(Error::Capacity("too many managed listeners"));
            }
            let version = existing
                .as_ref()
                .map_or(Some(1), |l| l.material_version.checked_add(1))
                .ok_or(Error::Capacity("material version exhausted"))?;
            let material = if let Some(vless) = &item.vless {
                let old = decrypt(&tx, network, &item.id, &self.material_key)?;
                Some(match (imported, old) {
                    (Some(material), _) => material,
                    (None, Some(mut old)) if old.settings.server_name == vless.server_name => {
                        old.version = version;
                        old.settings = vless.clone();
                        old
                    }
                    _ => generate(&item.id, version, vless)?,
                })
            } else {
                None
            };
            let (nonce, ciphertext) = if let Some(material) = &material {
                let random = local::credential()?;
                let nonce: Vec<u8> = (0..12)
                    .map(|i| u8::from_str_radix(&random[i * 2..i * 2 + 2], 16).expect("random hex"))
                    .collect();
                let bytes = Zeroizing::new(
                    serde_json::to_vec(material)
                        .map_err(|_| Error::Configuration("cannot encode material"))?,
                );
                let cipher = ChaCha20Poly1305::new_from_slice(&self.material_key)
                    .map_err(|_| Error::Configuration("invalid material key"))?;
                let ciphertext = cipher
                    .encrypt(
                        nonce.as_slice().into(),
                        Payload {
                            msg: &bytes,
                            aad: &aad(network, &item.id, &item.node_id, version),
                        },
                    )
                    .map_err(|_| Error::Configuration("cannot encrypt material"))?;
                (Some(nonce), Some(ciphertext))
            } else {
                (None, None)
            };
            let encoded = serde_json::to_string(item)
                .map_err(|_| Error::Configuration("cannot encode settings"))?;
            tx.execute("INSERT INTO managed_listeners(network_id,listener_id,node_id,settings,material_version,certificate_sha256,material_nonce,material_ciphertext) VALUES (?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(network_id,listener_id) DO UPDATE SET settings=excluded.settings,material_version=excluded.material_version,certificate_sha256=excluded.certificate_sha256,material_nonce=excluded.material_nonce,material_ciphertext=excluded.material_ciphertext",params![network.as_str(),item.id,item.node_id,encoded,version,material.as_ref().map(|m|&m.certificate_sha256),nonce,ciphertext])?;
            configuration::bump(&tx, network)?;
            super::publication::finish_change(
                &tx,
                network,
                super::publication::ChangeKind::Adjacency,
            )?;
        }
        let response = ManagedListenersSnapshot {
            revision: configuration::revision(&tx, network)?,
            listeners: listeners(&tx, network)?,
        };
        tx.commit()?;
        Ok(response)
    }
    pub fn agent_managed_connections(
        &self,
        token: &str,
    ) -> Result<AgentManagedConnectionsSnapshot, Error> {
        let network = self.node_network(token)?;
        let connections = self.agent_connections(token)?;
        let materials = materials(
            &self.connection,
            &network,
            &connections.payload,
            &self.material_key,
        )?;
        let network_id = network.as_str().to_owned();
        let digest = managed_digest(&network_id, &connections, &materials)
            .map_err(|_| Error::Configuration("cannot hash managed connections"))?;
        let snapshot = AgentManagedConnectionsSnapshot {
            network_id,
            connections,
            materials,
            digest,
        };
        if serde_json::to_vec(&snapshot)
            .map_err(|_| Error::Configuration("cannot encode managed snapshot"))?
            .len()
            > MAX_CONNECTIONS_BYTES
        {
            return Err(Error::Capacity("managed connections exceed 1 MiB"));
        }
        Ok(snapshot)
    }
    pub fn agent_managed_dataplane(
        &mut self,
        token: &str,
    ) -> Result<AgentManagedDataplaneSnapshot, Error> {
        // Store is exclusively owned by the request closure; no writes can
        // interleave this read with the following material projection.
        let network = self.node_network(token)?;
        let dataplane = self.agent_dataplane(token)?;
        let materials = materials(
            &self.connection,
            &network,
            &dataplane.payload.connections,
            &self.material_key,
        )?;
        let network_id = network.as_str().to_owned();
        let digest = managed_digest(&network_id, &dataplane, &materials)
            .map_err(|_| Error::Configuration("cannot hash managed dataplane"))?;
        let snapshot = AgentManagedDataplaneSnapshot {
            network_id,
            dataplane,
            materials,
            digest,
        };
        if serde_json::to_vec(&snapshot)
            .map_err(|_| Error::Configuration("cannot encode managed snapshot"))?
            .len()
            > MAX_DATAPLANE_BYTES
        {
            return Err(Error::Capacity("managed dataplane exceeds 2 MiB"));
        }
        Ok(snapshot)
    }
}

fn validate_import(
    settings: &ManagedListenerSettings,
    material: &ManagedVlessMaterial,
) -> Result<(), Error> {
    let invalid = || {
        Error::Unprocessable(
            "invalid private import material, certificate name, validity or key binding",
        )
    };
    if settings.transport != ConnectionTransport::Vless
        || material.listener_id != settings.id
        || material.credential_ref != reference(&settings.id)
        || material.version != 1
        || Some(&material.settings) != settings.vless.as_ref()
    {
        return Err(invalid());
    }
    if material.certificate_pem.len() > 32768
        || material
            .private_key_pem
            .as_ref()
            .is_none_or(|key| key.len() > 16384)
    {
        return Err(invalid());
    }
    let uuid = &material.uuid;
    if uuid.len() != 36
        || uuid.bytes().enumerate().any(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b != b'-'
            } else {
                !b.is_ascii_hexdigit()
            }
        })
    {
        return Err(invalid());
    }
    let certs = rustls_pemfile::certs(&mut std::io::Cursor::new(
        material.certificate_pem.as_bytes(),
    ))
    .collect::<Result<Vec<_>, _>>()
    .map_err(|_| invalid())?;
    if certs.is_empty() || certs.len() > 16 {
        return Err(invalid());
    }
    let fingerprint: String = Sha256::digest(certs[0].as_ref())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if fingerprint != material.certificate_sha256 {
        return Err(invalid());
    }
    let key = material.private_key_pem.as_ref().ok_or_else(invalid)?;
    let mut cursor = std::io::Cursor::new(key.as_bytes());
    let key = rustls_pemfile::private_key(&mut cursor)
        .map_err(|_| invalid())?
        .ok_or_else(invalid)?;
    if rustls_pemfile::private_key(&mut cursor)
        .map_err(|_| invalid())?
        .is_some()
    {
        return Err(invalid());
    }
    let provider = std::sync::Arc::new(rustls::crypto::ring::default_provider());
    rustls::ServerConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|_| invalid())?
        .with_no_client_auth()
        .with_single_cert(certs.clone(), key)
        .map_err(|_| invalid())?;
    let mut roots = rustls::RootCertStore::empty();
    roots
        .add(certs.last().expect("nonempty").clone())
        .map_err(|_| invalid())?;
    let verifier = rustls::client::WebPkiServerVerifier::builder_with_provider(
        std::sync::Arc::new(roots),
        provider,
    )
    .build()
    .map_err(|_| invalid())?;
    use rustls::client::danger::ServerCertVerifier;
    let name = rustls::pki_types::ServerName::try_from(material.settings.server_name.clone())
        .map_err(|_| invalid())?;
    verifier
        .verify_server_cert(
            &certs[0],
            &certs[1..],
            &name,
            &[],
            rustls::pki_types::UnixTime::now(),
        )
        .map_err(|_| invalid())?;
    Ok(())
}

pub(super) fn enroll_initial(
    connection: &Connection,
    network: &NetworkId,
    token: &str,
    node_id: &str,
    key: &[u8],
) -> Result<(), Error> {
    let json:Option<String>=connection.query_row("SELECT i.settings FROM managed_enrollment_intents i JOIN enrollment_tokens t ON t.network_id=i.network_id AND t.id=i.token_id WHERE i.network_id=?1 AND t.token_hash=?2",params![network.as_str(),local::digest(token)],|r|r.get(0)).optional()?;
    let Some(json) = json else {
        return Ok(());
    };
    let mut item: ManagedListenerSettings = serde_json::from_str(&json)
        .map_err(|_| Error::Configuration("invalid initial listener"))?;
    if !item.node_id.is_empty() {
        return Err(Error::Configuration("initial listener already bound"));
    }
    item.node_id = node_id.into();
    validate(&item)?;
    if listeners(connection, network)?.len() >= 128 {
        return Err(Error::Capacity("too many managed listeners"));
    }
    if connections::source(connection, network)?
        .listeners
        .iter()
        .any(|l| l.id == item.id)
        || listeners(connection, network)?
            .iter()
            .any(|l| l.settings.id == item.id)
    {
        return Err(Error::Conflict);
    }
    let material = item
        .vless
        .as_ref()
        .map(|v| generate(&item.id, 1, v))
        .transpose()?;
    let (nonce, ciphertext) = if let Some(material) = &material {
        let random = local::credential()?;
        let nonce: Vec<u8> = (0..12)
            .map(|i| u8::from_str_radix(&random[i * 2..i * 2 + 2], 16).expect("random hex"))
            .collect();
        let bytes = Zeroizing::new(
            serde_json::to_vec(material)
                .map_err(|_| Error::Configuration("cannot encode material"))?,
        );
        let cipher = ChaCha20Poly1305::new_from_slice(key)
            .map_err(|_| Error::Configuration("invalid material key"))?;
        let ciphertext = cipher
            .encrypt(
                nonce.as_slice().into(),
                Payload {
                    msg: &bytes,
                    aad: &aad(network, &item.id, node_id, 1),
                },
            )
            .map_err(|_| Error::Configuration("cannot encrypt material"))?;
        (Some(nonce), Some(ciphertext))
    } else {
        (None, None)
    };
    connection.execute("INSERT INTO managed_listeners(network_id,listener_id,node_id,settings,material_version,certificate_sha256,material_nonce,material_ciphertext) VALUES(?1,?2,?3,?4,1,?5,?6,?7)",params![network.as_str(),item.id,node_id,serde_json::to_string(&item).map_err(|_|Error::Configuration("cannot encode initial listener"))?,material.as_ref().map(|m|&m.certificate_sha256),nonce,ciphertext])?;
    Ok(())
}
