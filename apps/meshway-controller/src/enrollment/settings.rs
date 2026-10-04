//! Preflight user-supplied admission settings before issuing an enrollment key.
use crate::{configuration, execution::NodeError, secrets::Sealer};
use meshway_configuration::ConfigurationIssue;
use meshway_model::NetworkId;
use meshway_protocol::enrollment::InitialNodeSettings;
use rusqlite::Transaction;

fn invalid(path: String, message: impl Into<String>) -> NodeError {
    NodeError::InvalidSettings(vec![ConfigurationIssue {
        path,
        message: message.into(),
    }])
}

pub(crate) fn normalize(
    tx: &Transaction<'_>,
    network: NetworkId,
    settings: &mut InitialNodeSettings,
    sealer: &Sealer,
) -> Result<(), NodeError> {
    if let Some(hostname) = &mut settings.hostname {
        hostname.make_ascii_lowercase();
        if !hostname.is_ascii()
            || hostname.contains('.')
            || meshway_transport_spec::normalize_host(hostname).as_ref() != Ok(hostname)
        {
            return Err(invalid(
                "/initialSettings/hostname".into(),
                "expected a single DNS label",
            ));
        }
    }
    for (key, listener) in &mut settings.listeners {
        let path = format!(
            "/initialSettings/listeners/{}",
            key.replace('~', "~0").replace('/', "~1")
        );
        if key.is_empty() {
            return Err(invalid(path, "listener key must not be empty"));
        }
        listener
            .normalize()
            .map_err(|error| invalid(path.clone(), error.0))?;
        configuration::materials::prepare_listener_materials(tx, network, listener, sealer, false)
            .map_err(|error| match error {
                configuration::Error::Invalid(_) => invalid(
                    path,
                    "listener credentials or TLS certificate/key/name/validity are invalid",
                ),
                _ => NodeError::Storage,
            })?;
    }
    Ok(())
}
