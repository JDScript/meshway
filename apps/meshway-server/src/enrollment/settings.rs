//! Preflight user-supplied admission settings before issuing an enrollment key.
use crate::{configuration, error::Error, secrets::Sealer};
use meshway_configuration::{ConfigurationIssue, IssueCode};
use meshway_model::NetworkId;
use meshway_protocol::enrollment::InitialNodeSettings;
use rusqlite::Transaction;

fn invalid(issue: ConfigurationIssue) -> Error {
    Error::invalid_configuration(vec![issue])
}

pub(crate) fn normalize(
    tx: &Transaction<'_>,
    network: NetworkId,
    settings: &mut InitialNodeSettings,
    sealer: &Sealer,
) -> Result<(), Error> {
    if let Some(hostname) = &mut settings.hostname {
        hostname.make_ascii_lowercase();
        if !hostname.is_ascii()
            || hostname.contains('.')
            || meshway_transport_spec::normalize_host(hostname).as_ref() != Ok(hostname)
        {
            return Err(invalid(ConfigurationIssue::new(
                "/initialSettings/hostname",
                IssueCode::InvalidHostname,
            )));
        }
    }
    for (key, listener) in &mut settings.listeners {
        let path = format!(
            "/initialSettings/listeners/{}",
            key.replace('~', "~0").replace('/', "~1")
        );
        if key.is_empty() {
            return Err(invalid(ConfigurationIssue::new(
                path,
                IssueCode::EmptyListenerKey,
            )));
        }
        listener.normalize().map_err(|error| {
            invalid(
                ConfigurationIssue::new(&path, IssueCode::InvalidTransport)
                    .with("reason", error.code()),
            )
        })?;
        configuration::materials::prepare_listener_materials(tx, network, listener, sealer, false)
            .map_err(|error| configuration::materials::at_listener(error, &path))?;
    }
    Ok(())
}
