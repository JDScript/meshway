//! Server-owned material generation, certificate verification and private
//! irreversible-revocation checks. Declarations never self-attest validation.
use crate::accounts::random_uuid;
use crate::configuration::Snapshot;
use crate::error::Error;
use crate::secrets::Sealer;
use chrono::Datelike;
use meshway_configuration::{ConfigurationError, NetworkConfigurationInput};
use meshway_model::{NetworkId, NodeId};
use meshway_transport_spec::{Listener, ListenerTls, SecretString, TlsMode};
use rusqlite::{Transaction, params};
use std::collections::{BTreeMap, BTreeSet};

fn invalid() -> Error {
    Error::InvalidConfiguration(ConfigurationError::Invalid(vec![
        meshway_configuration::ConfigurationIssue::new(
            "",
            meshway_configuration::IssueCode::InvalidListenerMaterials,
        ),
    ]))
}
/// Places a listener's material diagnostics at that listener.
pub(crate) fn at_listener(error: Error, path: &str) -> Error {
    match error {
        Error::InvalidConfiguration(ConfigurationError::Invalid(issues)) => {
            Error::InvalidConfiguration(ConfigurationError::Invalid(
                issues
                    .into_iter()
                    .map(|mut issue| {
                        if issue.path.is_empty() {
                            issue.path = path.to_owned();
                        }
                        issue
                    })
                    .collect(),
            ))
        }
        other => other,
    }
}

pub(crate) fn prepare_materials(
    tx: &Transaction<'_>,
    network: NetworkId,
    input: &mut NetworkConfigurationInput,
    current: &Snapshot,
    sealer: &Sealer,
    generate: bool,
    recreated: &BTreeSet<(NodeId, String)>,
) -> Result<BTreeMap<(NodeId, String), Listener>, Error> {
    let mut verified = BTreeMap::new();
    for (node_id, node) in &mut input.nodes {
        for (key, listener) in &mut node.listeners {
            let previous = current
                .document
                .configuration
                .nodes
                .get(node_id)
                .and_then(|n| n.listeners.get(key))
                .map(|l| l.declaration());
            if !recreated.contains(&(*node_id, key.clone()))
                && let Some(previous) = previous
            {
                meshway_configuration::inherit_materials(listener, previous);
            }
            let prepared = prepare_listener_materials(tx, network, listener, sealer, generate)
                .map_err(|error| {
                    at_listener(
                        error,
                        &format!(
                            "/nodes/{node_id}/listeners/{}",
                            key.replace('~', "~0").replace('/', "~1")
                        ),
                    )
                })?;
            if prepared {
                verified.insert((*node_id, key.clone()), listener.clone());
            }
        }
    }
    Ok(verified)
}

/// Shared by configuration application and enrollment-key preflight. With
/// generation disabled, absent automatic material stays absent; supplied
/// material still receives complete certificate and irreversible-revocation checks.
pub(crate) fn prepare_listener_materials(
    tx: &Transaction<'_>,
    network: NetworkId,
    listener: &mut Listener,
    sealer: &Sealer,
    generate: bool,
) -> Result<bool, Error> {
    match listener {
        Listener::Vless(v) => {
            if v.uuid.is_none() && generate {
                v.uuid = Some(SecretString::new(random_uuid()?.to_string()));
            }
            prepare_tls(&mut v.tls, generate)?;
        }
        Listener::Trojan(v) => {
            if v.password.is_none() && generate {
                v.password = Some(SecretString::new(
                    crate::accounts::issue_one_time("mwp_")?
                        .0
                        .expose()
                        .to_owned(),
                ));
            }
            prepare_tls(&mut v.tls, generate)?;
        }
        _ => {}
    }
    listener.normalize().map_err(|error| {
        Error::InvalidConfiguration(ConfigurationError::Invalid(vec![
            meshway_configuration::ConfigurationIssue::new(
                "",
                meshway_configuration::IssueCode::InvalidTransport,
            )
            .with("reason", error.code()),
        ]))
    })?;
    let tls = match listener {
        Listener::Vless(v) => {
            if let Some(value) = &v.uuid {
                check_revocation(tx, sealer, network, "vlessUuid", value.expose().as_bytes())?;
            }
            &v.tls
        }
        Listener::Trojan(v) => {
            if let Some(value) = &v.password {
                check_revocation(
                    tx,
                    sealer,
                    network,
                    "trojanPassword",
                    value.expose().as_bytes(),
                )?;
            }
            &v.tls
        }
        _ => return Ok(false),
    };
    if tls.certificate.is_some() && tls.private_key.is_some() {
        let leaf = verify_tls(tls)?;
        check_revocation(tx, sealer, network, "tlsCertificate", &leaf)?;
        return Ok(true);
    }
    Ok(false)
}

fn prepare_tls(tls: &mut ListenerTls, generate: bool) -> Result<(), Error> {
    match (&tls.certificate, &tls.private_key) {
        (Some(_), Some(_)) => return Ok(()),
        (None, None) if tls.mode == TlsMode::Managed => {}
        _ => return Err(invalid()),
    }
    if !generate {
        return Ok(());
    }
    let mut params =
        rcgen::CertificateParams::new(vec![tls.server_name.clone()]).map_err(|_| invalid())?;
    let now = chrono::Utc::now().date_naive();
    let end = now
        .checked_add_signed(chrono::Duration::days(90))
        .ok_or(Error::Internal("certificate validity overflow"))?;
    params.not_before = rcgen::date_time_ymd(
        now.year(),
        u8::try_from(now.month()).map_err(|_| Error::Internal("certificate month"))?,
        u8::try_from(now.day()).map_err(|_| Error::Internal("certificate day"))?,
    );
    params.not_after = rcgen::date_time_ymd(
        end.year(),
        u8::try_from(end.month()).map_err(|_| Error::Internal("certificate month"))?,
        u8::try_from(end.day()).map_err(|_| Error::Internal("certificate day"))?,
    );
    let key = rcgen::KeyPair::generate().map_err(|_| Error::Internal("TLS key generation"))?;
    let certificate = params
        .self_signed(&key)
        .map_err(|_| Error::Internal("TLS certificate signing"))?;
    tls.certificate = Some(SecretString::new(certificate.pem()));
    tls.private_key = Some(SecretString::new(key.serialize_pem()));
    Ok(())
}

/// Real WebPKI certificate-chain, DNS/IP name, current-time validation plus
/// supported-provider SPKI/private-key consistency. No insecure verifier.
pub(crate) fn verify_tls(tls: &ListenerTls) -> Result<Vec<u8>, Error> {
    meshway_tls::verify_listener(tls).map_err(|_| invalid())
}

fn check_revocation(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    network: NetworkId,
    kind: &str,
    value: &[u8],
) -> Result<(), Error> {
    let fingerprint = sealer.fingerprint(network, kind, value)?;
    let revoked = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM credential_revocations WHERE network_id=?1 AND kind=?2 AND fingerprint=?3)",
        params![network.as_uuid().as_bytes().as_slice(), kind, fingerprint.as_slice()],
        |row| row.get::<_, bool>(0),
    )?;
    if revoked { Err(invalid()) } else { Ok(()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn managed() -> ListenerTls {
        serde_json::from_str(r#"{"mode":"managed","serverName":"node.example.test"}"#).unwrap()
    }

    #[test]
    fn generated_tls_verifies_but_wrong_name_key_and_expiry_fail() {
        let mut tls = managed();
        prepare_tls(&mut tls, true).unwrap();
        assert!(!verify_tls(&tls).unwrap().is_empty());
        let original = tls.clone();
        tls.server_name = "other.example.test".into();
        assert!(verify_tls(&tls).is_err());
        tls = original.clone();
        tls.private_key = Some(SecretString::new(
            rcgen::KeyPair::generate().unwrap().serialize_pem(),
        ));
        assert!(verify_tls(&tls).is_err());
        for (from, to) in [(2000, 2001), (2090, 2091)] {
            let mut params =
                rcgen::CertificateParams::new(vec![original.server_name.clone()]).unwrap();
            params.not_before = rcgen::date_time_ymd(from, 1, 1);
            params.not_after = rcgen::date_time_ymd(to, 1, 1);
            let key = rcgen::KeyPair::generate().unwrap();
            tls = original.clone();
            tls.certificate = Some(SecretString::new(params.self_signed(&key).unwrap().pem()));
            tls.private_key = Some(SecretString::new(key.serialize_pem()));
            assert!(verify_tls(&tls).is_err());
        }
        tls = original;
        tls.certificate = Some(SecretString::new("not a certificate".into()));
        assert!(verify_tls(&tls).is_err());
    }

    #[test]
    fn partial_material_never_silently_generates_a_replacement() {
        let mut tls = managed();
        prepare_tls(&mut tls, false).unwrap();
        assert!(tls.certificate.is_none());
        tls.private_key = Some(SecretString::new(
            rcgen::KeyPair::generate().unwrap().serialize_pem(),
        ));
        assert!(prepare_tls(&mut tls, true).is_err());
    }

    #[test]
    fn provided_tls_form_update_retains_only_same_resource_material() {
        let mut tls = managed();
        prepare_tls(&mut tls, true).unwrap();
        tls.mode = TlsMode::Provided;
        let mut old: Listener = serde_json::from_str(r#"{"type":"vless","listenPort":443,"tls":{"serverName":"node.example.test","mode":"provided"}}"#).unwrap();
        let Listener::Vless(v) = &mut old else {
            unreachable!()
        };
        v.tls = tls;
        v.uuid = Some(SecretString::new(random_uuid().unwrap().to_string()));
        let update: meshway_configuration::ListenerUpdate = serde_json::from_str(r#"{"type":"vless","listenPort":8443,"tls":{"serverName":"node.example.test","mode":"provided"}}"#).unwrap();
        let Listener::Vless(updated) = update.apply(Some(&old)) else {
            unreachable!()
        };
        assert_eq!(updated.listen_port.get(), 8443);
        verify_tls(&updated.tls).unwrap();
        let Listener::Vless(mut created) = update.apply(None) else {
            unreachable!()
        };
        assert!(prepare_tls(&mut created.tls, true).is_err());
        assert!(created.uuid.is_none());
    }
}
